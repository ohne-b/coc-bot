use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use opencv::{
    core::{Mat, Vector},
    imgcodecs,
    prelude::*,
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result,
    calibration::{Calibration, Region, Screen, asset_path},
    device::{
        Adb, InputMode,
        ldplayer::{self, Instance, LdPlayer},
        process::{Process, Transport},
    },
    settings::Settings,
    storage::Store,
    telemetry::*,
    vision::{Detection, Detector, Digits, crop, match_min, read_image},
};

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub settings: Settings,
    pub session: Option<Session>,
    pub detection: Detection,
    pub captured_at_ms: Option<i64>,
    pub frame_sequence: Option<u64>,
    pub resources: Vec<ResourceValue>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Installation {
    pub folder: PathBuf,
    pub instances: Vec<Instance>,
}

#[derive(Serialize)]
pub struct SessionDetail {
    pub session: Session,
    pub resources: Vec<ResourceValue>,
    pub home: Statistics,
    pub builder: Statistics,
    pub attacks: Vec<Attack>,
    pub walls: Vec<WallUpgrade>,
    pub activity: Vec<Activity>,
}

struct Frame {
    session_id: i64,
    sequence: u64,
    jpeg: Vec<u8>,
}

enum Command {
    Save(Settings, oneshot::Sender<Result<()>>),
    Discover(oneshot::Sender<Result<Vec<Installation>>>),
    Start(CancellationToken, oneshot::Sender<Result<()>>),
    Stop(oneshot::Sender<Result<()>>),
    History(Option<i64>, oneshot::Sender<Result<Vec<Session>>>),
    Detail(i64, oneshot::Sender<Result<SessionDetail>>),
}

/// One bounded worker owns device operations, OpenCV and SQLite, independently of the webview.
pub struct Application {
    commands: mpsc::Sender<Command>,
    snapshot: watch::Receiver<Snapshot>,
    frame: Arc<Mutex<Option<Frame>>>,
    shutdown: CancellationToken,
    control: tokio::sync::Mutex<CancellationToken>,
    worker: tokio::sync::Mutex<Option<JoinHandle<Result<()>>>>,
}

impl Application {
    pub async fn open(data: PathBuf, assets: PathBuf) -> Result<Self> {
        Self::with_transport(data, assets, Process).await
    }

    pub async fn with_transport<T: Transport + Clone + 'static>(
        data: PathBuf,
        assets: PathBuf,
        transport: T,
    ) -> Result<Self> {
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        let frame = Arc::new(Mutex::new(None));
        let worker_frame = frame.clone();
        let (commands, receiver) = mpsc::channel(16);
        let (snapshots, snapshot) = watch::channel(Snapshot {
            revision: 0,
            settings: Settings::default(),
            session: None,
            detection: Detection::Unknown,
            captured_at_ms: None,
            frame_sequence: None,
            resources: vec![],
            error: None,
        });
        let (ready, initialized) = oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let result = Worker::load(&data, &assets, transport, snapshots, worker_frame, token);
            match result {
                Ok(mut worker) => {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let _ = ready.send(Ok(()));
                    runtime.block_on(worker.run(receiver))
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                    Ok(())
                }
            }
        });
        if let Err(error) = initialized.await.map_err(|_| closed())? {
            shutdown.cancel();
            worker.await??;
            return Err(error);
        }
        Ok(Self {
            commands,
            snapshot,
            frame,
            control: tokio::sync::Mutex::new(shutdown.child_token()),
            shutdown,
            worker: tokio::sync::Mutex::new(Some(worker)),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.snapshot.borrow().clone();
        if self.commands.is_closed() && !self.shutdown.is_cancelled() {
            snapshot.error.get_or_insert_with(|| closed().to_string());
            if let Some(session) = &mut snapshot.session {
                session.state = SessionState::Failed;
            }
        }
        snapshot
    }

    pub fn screenshot(&self, session_id: i64, sequence: u64) -> Result<Vec<u8>> {
        let frame = self.frame.lock().map_err(|_| closed())?;
        let frame = frame
            .as_ref()
            .filter(|f| f.session_id == session_id && f.sequence == sequence)
            .ok_or_else(|| Error::Device("Screenshot has been superseded".into()))?;
        Ok(frame.jpeg.clone())
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| Error::Device("The application is busy or shutting down".into()))
    }

    pub async fn save_settings(&self, settings: Settings) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Save(settings, reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn discover(&self) -> Result<Vec<Installation>> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Discover(reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn start(&self) -> Result<()> {
        let mut token = self.control.lock().await;
        if self
            .snapshot()
            .session
            .is_some_and(|session| session.state.is_active())
        {
            return Err(Error::Device("A session is already running".into()));
        }
        let next = self.shutdown.child_token();
        let (reply, response) = oneshot::channel();
        *token = next.clone();
        self.send(Command::Start(next.clone(), reply))?;
        response.await.map_err(|_| closed())??;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let token = self.control.lock().await;
        token.cancel();
        let (reply, response) = oneshot::channel();
        self.send(Command::Stop(reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn history(&self, before: Option<i64>) -> Result<Vec<Session>> {
        let (reply, response) = oneshot::channel();
        self.send(Command::History(before, reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn detail(&self, id: i64) -> Result<SessionDetail> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Detail(id, reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.lock().await.take() {
            worker.await??;
        }
        Ok(())
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[derive(Deserialize)]
struct HudProfile {
    digits: PathBuf,
    capacity: BTreeMap<String, BTreeMap<String, HudRegion>>,
}

#[derive(Deserialize)]
struct HudRegion {
    bar: Region,
    #[serde(rename = "max")]
    maximum: Region,
    label: HudLabel,
}

#[derive(Deserialize)]
struct HudLabel {
    file: PathBuf,
    region: Region,
    max_score: f64,
    #[serde(skip)]
    image: Mat,
}

struct Running<T> {
    adb: Adb<T>,
    token: CancellationToken,
    session: Session,
    started: Instant,
}

struct Worker<T> {
    store: Store,
    calibration: Calibration,
    detector: Detector,
    digits: Digits,
    hud: HudProfile,
    transport: T,
    snapshot: watch::Sender<Snapshot>,
    frame: Arc<Mutex<Option<Frame>>>,
    shutdown: CancellationToken,
    running: Option<Running<T>>,
}

impl<T: Transport + Clone> Worker<T> {
    fn load(
        data: &Path,
        assets: &Path,
        transport: T,
        snapshot: watch::Sender<Snapshot>,
        frame: Arc<Mutex<Option<Frame>>>,
        shutdown: CancellationToken,
    ) -> Result<Self> {
        std::fs::create_dir_all(data)?;
        let mut store = Store::open(&data.join("history.sqlite3"))?;
        store.interrupt_open_sessions(now()?)?;
        let settings = store.settings()?;
        let calibration = Calibration::load(&assets.join("config.json"))?;
        let detector = Detector::load(&calibration, assets)?;
        let mut hud: HudProfile =
            serde_json::from_slice(&std::fs::read(assets.join("config.json"))?)?;
        for resources in hud.capacity.values_mut() {
            for region in resources.values_mut() {
                region.bar.validate(calibration.resolution)?;
                region.maximum.validate(calibration.resolution)?;
                region.label.region.validate(calibration.resolution)?;
                region.label.image = read_image(&asset_path(assets, &region.label.file)?)?;
                if !(0.0..=0.05).contains(&region.label.max_score)
                    || region.label.image.cols() > region.label.region.2 as i32
                    || region.label.image.rows() > region.label.region.3 as i32
                {
                    return Err(Error::Calibration("Invalid storage tooltip label".into()));
                }
            }
        }
        let digits = Digits::load(assets, &hud.digits)?;
        snapshot.send_modify(|s| {
            s.settings = settings;
            s.revision += 1;
        });
        Ok(Self {
            store,
            calibration,
            detector,
            digits,
            hud,
            transport,
            snapshot,
            frame,
            shutdown,
            running: None,
        })
    }

    async fn run(&mut self, mut commands: mpsc::Receiver<Command>) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => break,
                command = commands.recv() => match command {
                    Some(command) => self.command(command).await,
                    None => break,
                },
                _ = interval.tick(), if self.running.is_some() => {
                    if let Err(error) = self.observe().await {
                        if matches!(error, Error::Cancelled) {
                            self.finish(SessionState::Stopped)?;
                        } else {
                            let message = error.to_string();
                            if let Some(run) = &self.running {
                                let activity = Activity { sequence: 1, at_ms: now()?, kind: ActivityKind::Error, message: message.chars().take(400).collect() };
                                if let Err(storage) = self.store.record_activity(run.session.id, &activity) {
                                    self.publish_error(format!("{message}; history: {storage}"));
                                } else { self.publish_error(message); }
                            }
                            self.finish(SessionState::Failed)?;
                        }
                    }
                }
            }
        }
        self.finish(SessionState::Stopped)
    }

    fn publish_error(&self, message: String) {
        self.snapshot.send_modify(|s| {
            s.error = Some(message);
            s.revision += 1;
        });
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Save(settings, reply) => {
                let result = if self.running.is_some() {
                    Err(Error::Device(
                        "Stop the session before changing the emulator".into(),
                    ))
                } else {
                    self.store.save_settings(&settings).map(|()| {
                        self.snapshot.send_modify(|s| {
                            s.settings = settings;
                            s.revision += 1;
                        })
                    })
                };
                let _ = reply.send(result);
            }
            Command::Discover(reply) => {
                let result = self.discover().await;
                let _ = reply.send(result);
            }
            Command::Start(token, reply) => {
                let result = self.start(token);
                let _ = reply.send(result);
            }
            Command::Stop(reply) => {
                let result = self.finish(SessionState::Stopped);
                let _ = reply.send(result);
            }
            Command::History(before, reply) => {
                let _ = reply.send(self.store.sessions(before, 50));
            }
            Command::Detail(id, reply) => {
                let result = (|| {
                    Ok(SessionDetail {
                        session: self.store.session(id)?,
                        resources: self.store.resources(id)?,
                        home: self.store.statistics(id, Village::Home)?,
                        builder: self.store.statistics(id, Village::Builder)?,
                        attacks: self.store.attacks(id, None, 100)?,
                        walls: self.store.walls(id, None, 100)?,
                        activity: self.store.activity(id, None, 100)?,
                    })
                })();
                let _ = reply.send(result);
            }
        }
    }

    fn start(&mut self, token: CancellationToken) -> Result<()> {
        if self.running.is_some() {
            return Err(Error::Device("A session is already running".into()));
        }
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let settings = self.store.settings()?;
        let selected = settings
            .device
            .ok_or_else(|| Error::Device("Select an LDPlayer instance in Settings".into()))?;
        let adb = Adb::new(
            selected.config(self.calibration.resolution),
            InputMode::Observe,
            self.transport.clone(),
        )?;
        let session = self.store.begin_session(SessionMode::Observe, now()?)?;
        *self.frame.lock().map_err(|_| closed())? = None;
        self.snapshot.send_modify(|s| {
            s.revision += 1;
            s.session = Some(session.clone());
            s.detection = Detection::Unknown;
            s.captured_at_ms = None;
            s.frame_sequence = None;
            s.resources.clear();
            s.error = None;
        });
        self.running = Some(Running {
            adb,
            token,
            session,
            started: Instant::now(),
        });
        Ok(())
    }

    fn finish(&mut self, state: SessionState) -> Result<()> {
        if let Some(run) = self.running.take() {
            run.token.cancel();
            let result =
                self.store
                    .update_session(run.session.id, state, elapsed(run.started), now()?);
            match result {
                Ok(session) => self.snapshot.send_modify(|s| {
                    s.session = Some(session);
                    s.revision += 1;
                }),
                Err(error) => {
                    self.snapshot.send_modify(|s| {
                        if let Some(session) = &mut s.session {
                            session.state = SessionState::Failed;
                        }
                    });
                    self.publish_error(error.to_string());
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    async fn discover(&self) -> Result<Vec<Installation>> {
        if self.running.is_some() {
            return Err(Error::Device(
                "Stop the session before discovering emulators".into(),
            ));
        }
        let configured = self.store.settings()?.device.map(|d| d.folder);
        let roots = (b'C'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", char::from(letter))))
            .collect();
        let folders =
            ldplayer::installed_folders(configured, roots, &self.transport, &self.shutdown).await?;
        let mut installations = Vec::new();
        for folder in folders.into_iter().take(16) {
            let player = LdPlayer::new(
                folder.join("ldconsole.exe"),
                0,
                InputMode::Observe,
                self.transport.clone(),
            )?;
            installations.push(Installation {
                instances: player.instances(&self.shutdown).await?,
                folder,
            });
        }
        Ok(installations)
    }

    async fn observe(&mut self) -> Result<()> {
        let run = self.running.as_mut().ok_or_else(closed)?;
        run.adb.verify(&run.token).await?;
        let at = now()?;
        let capture = run.adb.capture(&run.token).await?;
        let detection = self.detector.detect(&capture.image)?;
        let village = match detection {
            Detection::Known(Screen::Home) => Some((Village::Home, "home")),
            Detection::Known(Screen::Builder) => Some((Village::Builder, "builder")),
            _ => None,
        };
        if let Some((village, name)) = village
            && let Some(regions) = self.hud.capacity.get(name)
        {
            for (resource, key) in [
                (Resource::Gold, "gold"),
                (Resource::Elixir, "elixir"),
                (Resource::Dark, "dark"),
            ] {
                if let Some(region) = regions.get(key) {
                    let capacity = if match_min(
                        &crop(&capture.image, region.label.region)?,
                        &region.label.image,
                    )? <= region.label.max_score
                    {
                        self.digits
                            .number(&crop(&capture.image, region.maximum)?, 0.65)
                            .ok()
                            .and_then(|value| u32::try_from(value).ok())
                            .filter(|value| *value > 0)
                    } else {
                        None
                    };
                    let reading = self
                        .digits
                        .trailing(&crop(&capture.image, region.bar)?, 0.65, 110)
                        .ok()
                        .and_then(|value| u32::try_from(value).ok())
                        .map(|amount| {
                            Reading::Known(Balance {
                                amount,
                                capacity: capacity.filter(|maximum| *maximum >= amount),
                            })
                        })
                        .unwrap_or(Reading::Failed);
                    self.store.observe(
                        run.session.id,
                        ResourceObservation {
                            village,
                            resource,
                            checked_at_ms: at,
                            reading,
                        },
                    )?;
                }
            }
        }
        let mut jpeg = Vector::<u8>::new();
        imgcodecs::imencode(
            ".jpg",
            &capture.image,
            &mut jpeg,
            &Vector::from_slice(&[imgcodecs::IMWRITE_JPEG_QUALITY, 85]),
        )?;
        if run.token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let session = self.store.update_session(
            run.session.id,
            SessionState::Running,
            elapsed(run.started),
            now()?,
        )?;
        let resources = self.store.resources(run.session.id)?;
        let sequence = self.snapshot.borrow().revision + 1;
        *self.frame.lock().map_err(|_| closed())? = Some(Frame {
            session_id: run.session.id,
            sequence,
            jpeg: jpeg.to_vec(),
        });
        self.snapshot.send_modify(|s| {
            s.revision = sequence;
            s.session = Some(session);
            s.detection = detection;
            s.captured_at_ms = Some(at);
            s.frame_sequence = Some(sequence);
            s.resources = resources;
        });
        Ok(())
    }
}

fn elapsed(start: Instant) -> i64 {
    start.elapsed().as_millis().min(9_007_199_254_740_991) as i64
}
fn now() -> Result<i64> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::History("System clock precedes 1970".into()))?
        .as_millis();
    i64::try_from(value)
        .map_err(|_| Error::History("System clock is outside the supported range".into()))
}
fn closed() -> Error {
    Error::Device("The application worker has stopped".into())
}
