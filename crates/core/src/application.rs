use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use opencv::{core::Vector, imgcodecs, prelude::*};
use serde::Serialize;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result,
    calibration::{Calibration, Screen},
    device::{
        Adb, InputMode,
        ldplayer::{self, Instance, LdPlayer},
        process::{Process, Transport},
    },
    gameplay::{Engine, Progress, profile::Profile},
    settings::{Settings, SettingsUpdate},
    storage::Store,
    telemetry::*,
    vision::{Detection, Detector, Digits, army::Layout, crop},
};

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub army: Vec<crate::vision::army::recipe::Unit>,
    pub army_complete: bool,
    pub army_screen: Option<Screen>,
    pub army_observed_at_ms: Option<i64>,
    pub army_scanning: bool,
    pub revision: u64,
    pub settings: Settings,
    pub session: Option<Session>,
    pub detection: Detection,
    pub captured_at_ms: Option<i64>,
    pub frame_sequence: Option<u64>,
    pub resources: Vec<ResourceValue>,
    pub error: Option<String>,
    pub progress: Option<Progress>,
    pub statistics: Vec<Statistics>,
    pub default_layouts: BTreeMap<Screen, Layout>,
}

impl Snapshot {
    fn set_settings(&mut self, settings: Settings) {
        if self.settings.device != settings.device {
            self.army.clear();
            self.army_complete = false;
            self.army_screen = None;
            self.army_observed_at_ms = None;
        }
        self.settings = settings;
        self.revision += 1;
    }
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
    Update(SettingsUpdate, oneshot::Sender<Result<()>>),
    Discover(Option<PathBuf>, oneshot::Sender<Result<Vec<Installation>>>),
    Start(SessionMode, CancellationToken, oneshot::Sender<Result<()>>),
    ScanArmy(Screen, CancellationToken, oneshot::Sender<Result<()>>),
    Pause(bool, oneshot::Sender<Result<()>>),
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
    control: tokio::sync::Mutex<Control>,
    worker: tokio::sync::Mutex<Option<JoinHandle<Result<()>>>>,
}

struct Control {
    token: CancellationToken,
    scanning: bool,
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
            army: vec![],
            army_complete: false,
            army_screen: None,
            army_observed_at_ms: None,
            army_scanning: false,
            revision: 0,
            settings: Settings::default(),
            session: None,
            detection: Detection::Unknown,
            captured_at_ms: None,
            frame_sequence: None,
            resources: vec![],
            error: None,
            progress: None,
            statistics: vec![],
            default_layouts: BTreeMap::new(),
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
            control: tokio::sync::Mutex::new(Control {
                token: shutdown.child_token(),
                scanning: false,
            }),
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

    pub async fn update_settings(&self, update: SettingsUpdate) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Update(update, reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn discover(&self, folder: Option<PathBuf>) -> Result<Vec<Installation>> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Discover(folder, reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn start(&self, mode: SessionMode) -> Result<()> {
        let mut token = self.control.lock().await;
        if (token.scanning && !token.token.is_cancelled())
            || self
                .snapshot()
                .session
                .is_some_and(|session| session.state.is_active())
        {
            return Err(Error::Device("A session is already running".into()));
        }
        let next = self.shutdown.child_token();
        let (reply, response) = oneshot::channel();
        token.token = next.clone();
        token.scanning = false;
        self.send(Command::Start(mode, next.clone(), reply))?;
        response.await.map_err(|_| closed())??;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let token = self.control.lock().await;
        token.token.cancel();
        let (reply, response) = oneshot::channel();
        self.send(Command::Stop(reply))?;
        response.await.map_err(|_| closed())?
    }

    pub async fn scan_army(&self, screen: Screen) -> Result<()> {
        if !matches!(
            screen,
            Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2
        ) {
            return Err(Error::InputGuard);
        }
        let mut control = self.control.lock().await;
        if (control.scanning && !control.token.is_cancelled())
            || self.snapshot().session.is_some_and(|s| s.state.is_active())
        {
            return Err(Error::Device(
                "Stop the session before reading the army".into(),
            ));
        }
        let token = self.shutdown.child_token();
        let (reply, response) = oneshot::channel();
        self.send(Command::ScanArmy(screen, token.clone(), reply))?;
        control.token = token;
        control.scanning = true;
        drop(control);
        response.await.map_err(|_| closed())?
    }

    pub fn preview_deployment(
        &self,
        screen: Screen,
    ) -> Result<crate::vision::army::recipe::Recipe> {
        let snapshot = self.snapshot();
        if !matches!(
            screen,
            Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2
        ) || snapshot.army_screen != Some(screen)
            || !snapshot.army_complete
            || snapshot.army_scanning
        {
            return Err(Error::Gameplay(
                "Read this stage's army before creating a plan".into(),
            ));
        }
        let options = &snapshot.settings.farming;
        crate::vision::army::automatic::generate(
            &snapshot.army,
            options.strategy == crate::gameplay::Strategy::Ring,
            screen != Screen::HomeScout,
            options.builder_stack,
            options.earthquake,
        )
    }

    pub async fn pause(&self, paused: bool) -> Result<()> {
        let (reply, response) = oneshot::channel();
        self.send(Command::Pause(paused, reply))?;
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

struct Running<T> {
    adb: Adb<T>,
    token: CancellationToken,
    session: Session,
    started: Instant,
    active_ms: i64,
    paused: bool,
    engine: Option<Engine>,
    activity_sequence: u32,
    player: LdPlayer<T>,
    recovering: bool,
    guard_failures: u8,
    next_memory_check: Instant,
}
impl<T> Running<T> {
    fn elapsed(&self) -> i64 {
        self.active_ms
            + if self.paused {
                0
            } else {
                elapsed(self.started)
            }
    }
}

struct Worker<T> {
    store: Store,
    calibration: Calibration,
    detector: Arc<Detector>,
    digits: Digits,
    profile: Profile,
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
        let detector = Arc::new(Detector::load(&calibration, assets)?);
        let profile = Profile::load(assets)?;
        let digits = Digits::load(assets, &profile.digits)?;
        snapshot.send_modify(|s| {
            s.settings = settings;
            s.default_layouts = profile
                .screens
                .iter()
                .filter_map(|(screen, spec)| spec.army_bar.map(|_| (*screen, Layout::default())))
                .collect();
            s.revision += 1;
        });
        Ok(Self {
            store,
            calibration,
            detector,
            digits,
            profile,
            transport,
            snapshot,
            frame,
            shutdown,
            running: None,
        })
    }

    async fn run(&mut self, mut commands: mpsc::Receiver<Command>) -> Result<()> {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        'worker: loop {
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => break,
                command = commands.recv() => match command {
                    Some(command) => self.command(command).await,
                    None => break,
                },
                _ = interval.tick(), if self.running.is_some() => {
                    if let Err(mut error) = self.observe().await {
                        loop {
                            match self.recover(&error).await {
                                Ok(true) => continue 'worker,
                                Ok(false) => break,
                                Err(next) => {
                                    let retry = recoverable(&next);
                                    error = next;
                                    if !retry { break; }
                                }
                            }
                        }
                        if matches!(error, Error::Cancelled) {
                            self.finish(SessionState::Stopped)?;
                        } else {
                            let message = error.to_string();
                            if let Some(run) = &self.running {
                                let activity = Activity { sequence: run.activity_sequence+1, at_ms: now()?, kind: ActivityKind::Error, message: message.chars().take(400).collect() };
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
            Command::Update(update, reply) => {
                let result = (|| {
                    if self.running.is_some() {
                        return Err(Error::Device(
                            "Stop the session before changing settings".into(),
                        ));
                    }
                    let mut settings = self.store.settings()?;
                    settings.apply(update)?;
                    self.store.save_settings(&settings)?;
                    self.snapshot.send_modify(|s| {
                        s.set_settings(settings);
                    });
                    Ok(())
                })();
                let _ = reply.send(result);
            }
            Command::Save(settings, reply) => {
                let result = if self.running.is_some() {
                    Err(Error::Device(
                        "Stop the session before changing the emulator".into(),
                    ))
                } else {
                    self.store.save_settings(&settings).map(|()| {
                        self.snapshot.send_modify(|s| {
                            s.set_settings(settings);
                        })
                    })
                };
                let _ = reply.send(result);
            }
            Command::Discover(folder, reply) => {
                let result = self.discover(folder).await;
                let _ = reply.send(result);
            }
            Command::Start(mode, token, reply) => {
                let result = self.start(mode, token);
                let _ = reply.send(result);
            }
            Command::ScanArmy(screen, token, reply) => {
                self.snapshot.send_modify(|s| {
                    s.army_scanning = true;
                    s.revision += 1;
                });
                let result = self.scan_army(screen, &token).await;
                token.cancel();
                self.snapshot.send_modify(|s| {
                    s.army_scanning = false;
                    s.revision += 1;
                });
                let _ = reply.send(result);
            }
            Command::Pause(paused, reply) => {
                let result = self.pause(paused);
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

    fn start(&mut self, mode: SessionMode, token: CancellationToken) -> Result<()> {
        if self.running.is_some() {
            return Err(Error::Device("A session is already running".into()));
        }
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let settings = self.store.settings()?;
        let mode = if mode == SessionMode::Farming && settings.farming.upgrade_walls {
            SessionMode::Walls
        } else {
            mode
        };
        let selected = settings
            .device
            .ok_or_else(|| Error::Device("Select an LDPlayer instance in Settings".into()))?;
        let engine = if mode == SessionMode::Observe {
            None
        } else {
            Some(Engine::new(settings.farming, mode, &self.profile)?)
        };
        let input = if mode == SessionMode::Observe {
            InputMode::Observe
        } else {
            InputMode::Live
        };
        let player = LdPlayer::new(
            selected.folder.join("ldconsole.exe"),
            selected.index,
            input,
            self.transport.clone(),
        )?;
        let adb = Adb::new(
            selected.config(self.calibration.resolution),
            input,
            self.transport.clone(),
        )?;
        let session = self.store.begin_session(mode, now()?)?;
        *self.frame.lock().map_err(|_| closed())? = None;
        self.snapshot.send_modify(|s| {
            s.revision += 1;
            s.army.clear();
            s.army_complete = false;
            s.army_screen = None;
            s.army_observed_at_ms = None;
            s.session = Some(session.clone());
            s.detection = Detection::Unknown;
            s.captured_at_ms = None;
            s.frame_sequence = None;
            s.resources.clear();
            s.error = None;
            s.progress = engine.as_ref().map(Engine::progress);
            s.statistics.clear();
        });
        self.running = Some(Running {
            adb,
            token,
            session,
            started: Instant::now(),
            active_ms: 0,
            paused: false,
            engine,
            activity_sequence: 0,
            player,
            recovering: false,
            guard_failures: 0,
            next_memory_check: Instant::now() + Duration::from_secs(1800),
        });
        Ok(())
    }

    async fn scan_army(&mut self, screen: Screen, token: &CancellationToken) -> Result<()> {
        use crate::vision::army::roster::{ScanStep, Scanner};
        if self.running.is_some() {
            return Err(Error::Device(
                "Stop the session before reading the army".into(),
            ));
        }
        let selected = self
            .store
            .settings()?
            .device
            .ok_or_else(|| Error::Device("Select an LDPlayer instance in Settings".into()))?;
        let mut adb = Adb::new(
            selected.config(self.calibration.resolution),
            InputMode::Live,
            self.transport.clone(),
        )?;
        let bar = self
            .profile
            .screen(screen)?
            .army_bar
            .ok_or(Error::InputGuard)?;
        let mut scanner = Scanner::new(bar, screen != Screen::HomeScout)?;
        let allowed = crate::gameplay::deploy_screens(screen);
        let mut started = false;
        let completed = std::cell::Cell::new(false);
        let guard = |image: &Mat| -> Result<()> {
            if self.detector.detect(image)? == Detection::Known(Screen::BuilderBattle)
                && !crate::gameplay::builder_stage_ready(
                    &self.profile,
                    &self.digits,
                    Some(screen),
                    image,
                    &completed,
                )?
            {
                return Err(Error::InputGuard);
            }
            Ok(())
        };
        adb.verify(token).await?;
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if token.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let frame = adb.capture(token).await?;
            let detection = self.detector.detect(&frame.image)?;
            if !matches!(detection, Detection::Known(s) if if started {allowed.contains(&s)} else {s==screen})
            {
                return Err(Error::Gameplay(
                    "Open the matching opponent scout screen to read its army".into(),
                ));
            }
            guard(&frame.image)?;
            started = true;
            match scanner.observe(&frame.image, &self.digits)? {
                ScanStep::Wait => {
                    tokio::select! {_=token.cancelled()=>return Err(Error::Cancelled),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
                }
                ScanStep::Pan(pan) => {
                    adb.gesture_checked(
                        self.detector.clone(),
                        &allowed,
                        |frame| {
                            guard(frame)?;
                            Ok((
                                crate::device::Gesture::Drag {
                                    from: pan.from,
                                    to: pan.to,
                                },
                                (),
                            ))
                        },
                        token,
                    )
                    .await?;
                    tokio::select! {_=token.cancelled()=>return Err(Error::Cancelled),_=tokio::time::sleep(Duration::from_millis(350))=>{}}
                }
                ScanStep::Complete(roster) => {
                    let current = adb.capture(token).await?;
                    if !matches!(self.detector.detect(&current.image)?, Detection::Known(s) if allowed.contains(&s))
                    {
                        return Err(Error::InputGuard);
                    }
                    guard(&current.image)?;
                    let at = now()?;
                    self.snapshot.send_modify(|s| {
                        s.army = roster.inventory();
                        s.army_complete = true;
                        s.army_screen = Some(screen);
                        s.army_observed_at_ms = Some(at);
                        s.revision += 1;
                    });
                    return Ok(());
                }
            }
        }
    }

    fn pause(&mut self, paused: bool) -> Result<()> {
        let run = self
            .running
            .as_mut()
            .ok_or_else(|| Error::Device("No session is running".into()))?;
        if run.paused == paused {
            return Ok(());
        }
        let active_ms = run.elapsed();
        let session = self.store.update_session(
            run.session.id,
            if paused {
                SessionState::Paused
            } else {
                SessionState::Running
            },
            active_ms,
            now()?,
        )?;
        run.active_ms = active_ms;
        run.started = Instant::now();
        run.paused = paused;
        self.snapshot.send_modify(|s| {
            s.session = Some(session);
            s.revision += 1;
        });
        Ok(())
    }

    async fn recover(&mut self, error: &Error) -> Result<bool> {
        let Some(run) = self.running.as_mut() else {
            return Ok(false);
        };
        if run.engine.is_none() || run.paused || run.token.is_cancelled() {
            return Ok(false);
        }
        if run
            .engine
            .as_ref()
            .is_some_and(|engine| !engine.recovery_safe())
        {
            return Ok(false);
        }
        if matches!(error, Error::InputGuard) {
            run.guard_failures += 1;
            return Ok(run.guard_failures <= 3);
        }
        if !recoverable(error) {
            return Ok(false);
        }
        let active_ms = run.elapsed();
        let attempt = run
            .engine
            .as_mut()
            .ok_or_else(closed)?
            .recover(active_ms as u64)?;
        run.activity_sequence += 1;
        self.store.record_activity(
            run.session.id,
            &Activity {
                sequence: run.activity_sequence,
                at_ms: now()?,
                kind: ActivityKind::RecoveryStarted,
                message: format!("Recovery {attempt}: {error}")
                    .chars()
                    .take(400)
                    .collect(),
            },
        )?;
        run.recovering = true;
        self.snapshot.send_modify(|s| {
            if let Some(progress) = &mut s.progress {
                progress.operation = "Recovering".into();
            }
            s.revision += 1;
        });
        let memory_failure = matches!(error, Error::LowMemory)
            || matches!(error, Error::OpenCv(e) if e.code == opencv::core::StsNoMem);
        if memory_failure {
            tokio::select! {_ = run.token.cancelled()=>return Err(Error::Cancelled),_ = tokio::time::sleep(Duration::from_secs(30))=>{}}
        }
        let probe = if memory_failure {
            Err(Error::LowMemory)
        } else {
            async {
                run.adb.verify(&run.token).await?;
                run.adb.capture(&run.token).await
            }
            .await
        };
        let capture = match probe {
            Ok(capture) => capture,
            Err(error) if !recoverable(&error) => return Err(error),
            Err(_) => {
                if run.player.running(&run.token).await? {
                    run.player.quit(&run.token).await?;
                    let deadline = Instant::now() + Duration::from_secs(60);
                    while run.player.running(&run.token).await? {
                        if Instant::now() >= deadline {
                            return Err(Error::Timeout);
                        }
                        tokio::select! {_ = run.token.cancelled()=>return Err(Error::Cancelled),_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
                    }
                }
                tokio::select! {_ = run.token.cancelled()=>return Err(Error::Cancelled),_ = tokio::time::sleep(Duration::from_secs(3))=>{}}
                run.player.launch(&run.token).await?;
                run.next_memory_check = Instant::now() + Duration::from_secs(1800);
                let deadline = Instant::now() + Duration::from_secs(90);
                loop {
                    match run.adb.verify(&run.token).await {
                        Ok(_) => break,
                        Err(error) if !recoverable(&error) => return Err(error),
                        Err(_) if Instant::now() < deadline => {}
                        Err(error) => return Err(error),
                    }
                    tokio::select! {_ = run.token.cancelled()=>return Err(Error::Cancelled),_ = tokio::time::sleep(Duration::from_secs(2))=>{}}
                }
                run.adb.capture(&run.token).await?
            }
        };
        if !run.adb.game_in_front(&run.token).await? {
            run.adb.launch_game(&run.token).await?;
        } else {
            match self.detector.detect(&capture.image)? {
                Detection::Known(
                    screen @ (Screen::Disconnected | Screen::Idle | Screen::SearchFailed),
                ) => {
                    let name = match screen {
                        Screen::Disconnected => "retry",
                        Screen::Idle => "reload",
                        _ => "close",
                    };
                    run.adb
                        .tap_checked(
                            self.detector.clone(),
                            &[screen],
                            |frame| Ok((self.profile.action(screen, name, frame)?, ())),
                            &run.token,
                        )
                        .await?;
                }
                Detection::Unknown if attempt >= 2 => {
                    run.adb.stop_game(&run.token).await?;
                    run.adb.launch_game(&run.token).await?;
                }
                _ => {}
            }
        }
        Ok(true)
    }

    fn finish(&mut self, state: SessionState) -> Result<()> {
        if let Some(run) = self.running.take() {
            run.token.cancel();
            let result = self
                .store
                .update_session(run.session.id, state, run.elapsed(), now()?);
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

    async fn discover(&self, folder: Option<PathBuf>) -> Result<Vec<Installation>> {
        if self.running.is_some() {
            return Err(Error::Device(
                "Stop the session before discovering emulators".into(),
            ));
        }
        if folder.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(Error::Device("Choose an absolute LDPlayer folder".into()));
        }
        let configured = folder.or(self.store.settings()?.device.map(|d| d.folder));
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
        if run.recovering && matches!(detection, Detection::Known(Screen::Home | Screen::Builder)) {
            run.activity_sequence += 1;
            self.store.record_activity(
                run.session.id,
                &Activity {
                    sequence: run.activity_sequence,
                    at_ms: at,
                    kind: ActivityKind::RecoveryCompleted,
                    message: "Village recovered".into(),
                },
            )?;
            run.recovering = false;
        }
        if !run.paused
            && matches!(detection, Detection::Known(Screen::Home | Screen::Builder))
            && run
                .engine
                .as_ref()
                .is_some_and(Engine::can_restart_between_attacks)
            && Instant::now() >= run.next_memory_check
        {
            run.next_memory_check = Instant::now() + Duration::from_secs(60);
            if ldplayer::available_memory_mb(&self.transport, &run.token)
                .await?
                .is_some_and(|mb| mb < 1000)
            {
                return Err(Error::LowMemory);
            }
        }
        let mut observations = Vec::new();
        let village = match detection {
            Detection::Known(Screen::Home) => Some((Village::Home, "home")),
            Detection::Known(Screen::Builder) => Some((Village::Builder, "builder")),
            _ => None,
        };
        if let Some((village, name)) = village
            && let Some(regions) = self.profile.capacity.get(name)
        {
            let mut tooltip_row = None;
            for region in regions.values() {
                if region.label.locate(&capture.image)?.is_some() {
                    tooltip_row =
                        Some(tooltip_row.map_or(region.bar.1, |row: u32| row.min(region.bar.1)));
                }
            }
            for (resource, key) in [
                (Resource::Gold, "gold"),
                (Resource::Elixir, "elixir"),
                (Resource::Dark, "dark"),
            ] {
                if let Some(region) = regions.get(key) {
                    // A storage tooltip covers lower HUD rows with its maximum and treasury text.
                    if tooltip_row.is_some_and(|row| region.bar.1 > row) {
                        observations.push(ResourceObservation {
                            village,
                            resource,
                            checked_at_ms: at,
                            reading: Reading::Unavailable,
                        });
                        continue;
                    }
                    let capacity = if region.label.locate(&capture.image)?.is_some() {
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
                    observations.push(ResourceObservation {
                        village,
                        resource,
                        checked_at_ms: at,
                        reading,
                    });
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
        let active_ms = run.elapsed();
        let outcome = if !run.paused
            && let Some(engine) = &mut run.engine
        {
            engine
                .tick(
                    &crate::gameplay::GameVision {
                        profile: &self.profile,
                        digits: &self.digits,
                        detector: self.detector.clone(),
                    },
                    &mut run.adb,
                    &crate::gameplay::FrameView {
                        image: &capture.image,
                        detection: &detection,
                        elapsed_ms: active_ms as u64,
                        captured_at_ms: at,
                    },
                    &run.token,
                )
                .await?
        } else {
            Default::default()
        };
        run.guard_failures = 0;
        for observation in outcome.resources {
            observations.retain(|old| {
                old.village != observation.village || old.resource != observation.resource
            });
            observations.push(observation);
        }
        for observation in observations {
            self.store.observe(run.session.id, observation)?;
        }
        if let Some(attack) = outcome.attack {
            self.store.record_attack(run.session.id, &attack)?;
        }
        if let Some(wall) = outcome.wall {
            self.store.record_wall(run.session.id, &wall)?;
        }
        let session = self.store.update_session(
            run.session.id,
            if run.paused {
                SessionState::Paused
            } else {
                SessionState::Running
            },
            run.elapsed(),
            now()?,
        )?;
        let resources = self.store.resources(run.session.id)?;
        let statistics = vec![
            self.store.statistics(run.session.id, Village::Home)?,
            self.store.statistics(run.session.id, Village::Builder)?,
        ];
        let sequence = self.snapshot.borrow().revision + 1;
        *self.frame.lock().map_err(|_| closed())? = Some(Frame {
            session_id: run.session.id,
            sequence,
            jpeg: jpeg.to_vec(),
        });
        self.snapshot.send_modify(|s| {
            s.revision = sequence;
            if let Some((screen, units)) = outcome.army {
                s.army = units;
                s.army_complete = true;
                s.army_screen = Some(screen);
                s.army_observed_at_ms = Some(at);
            }
            s.session = Some(session);
            s.detection = detection;
            s.captured_at_ms = Some(at);
            s.frame_sequence = Some(sequence);
            s.resources = resources;
            s.progress = run.engine.as_ref().map(Engine::progress);
            s.statistics = statistics;
        });
        if outcome.completed {
            self.finish(SessionState::Completed)?;
        }
        Ok(())
    }
}

fn elapsed(start: Instant) -> i64 {
    start.elapsed().as_millis().min(9_007_199_254_740_991) as i64
}
fn recoverable(error: &Error) -> bool {
    matches!(
        error,
        Error::Timeout
            | Error::Device(_)
            | Error::Io(_)
            | Error::Frame
            | Error::Unreadable
            | Error::LowMemory
    ) || matches!(error, Error::OpenCv(e) if e.code == opencv::core::StsNoMem)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        device::process::Invocation,
        gameplay::test_device::{Fake, root},
        settings::SelectedDevice,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct WaitingCapture {
        base: Fake,
        entered: Arc<tokio::sync::Notify>,
    }
    impl Transport for WaitingCapture {
        async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
            if command.args.last().is_some_and(|s| s == "screencap") {
                self.entered.notify_one();
                cancel.cancelled().await;
                return Err(Error::Cancelled);
            }
            self.base.run(command, cancel).await
        }
    }

    #[tokio::test]
    async fn army_read_is_owned_cancellable_and_creates_no_farming_session() {
        let (base, _) = Fake::new("english-home.png");
        let transport = WaitingCapture {
            base,
            entered: Arc::default(),
        };
        let data = tempfile::tempdir().unwrap();
        for file in ["adb.exe", "ldconsole.exe"] {
            std::fs::write(data.path().join(file), []).unwrap();
        }
        let app = Arc::new(
            Application::with_transport(data.path().to_owned(), root(), transport.clone())
                .await
                .unwrap(),
        );
        app.save_settings(Settings {
            device: Some(SelectedDevice {
                folder: data.path().to_owned(),
                index: 0,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
        for abandon in [false, true] {
            let task = {
                let app = app.clone();
                tokio::spawn(async move { app.scan_army(Screen::HomeScout).await })
            };
            tokio::time::timeout(Duration::from_secs(3), transport.entered.notified())
                .await
                .unwrap();
            assert!(app.snapshot().army_scanning);
            assert!(app.start(SessionMode::Observe).await.is_err());
            assert!(app.scan_army(Screen::BuilderScout).await.is_err());
            if abandon {
                task.abort();
            }
            tokio::time::timeout(Duration::from_secs(3), app.stop())
                .await
                .unwrap()
                .unwrap();
            if !abandon {
                assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
            }
            assert!(!app.snapshot().army_scanning);
            assert!(app.snapshot().army.is_empty());
            assert!(app.history(None).await.unwrap().is_empty());
        }
        assert!(transport.base.taps.lock().unwrap().is_empty());
        app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn manual_builder_army_scan_never_publishes_across_a_stage_transition() {
        #[derive(Clone)]
        struct ChangingCapture {
            before: Fake,
            after: Fake,
            captures: Arc<AtomicUsize>,
        }
        impl Transport for ChangingCapture {
            async fn run(
                &self,
                command: Invocation,
                cancel: &CancellationToken,
            ) -> Result<Vec<u8>> {
                if command.args.last().is_some_and(|s| s == "screencap")
                    && self.captures.fetch_add(1, Ordering::SeqCst) > 0
                {
                    return self.after.run(command, cancel).await;
                }
                self.before.run(command, cancel).await
            }
        }
        let (_, mut scout) = Fake::new("english-builder_current.png");
        let (_, army) = Fake::new("deployment-builder-before.png");
        let rect = opencv::core::Rect::new(0, 588, 1280, 120);
        Mat::roi(&army, rect)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut scout, rect).unwrap())
            .unwrap();
        for (source, before) in [
            (Screen::BuilderScout, scout.clone()),
            (
                Screen::BuilderStage2,
                Fake::new("battlefield-builder-second-active.png").1,
            ),
        ] {
            let transport = ChangingCapture {
                before: Fake::from_image(&before),
                after: Fake::new("battlefield-builder-second-active.png").0,
                captures: Arc::default(),
            };
            let data = tempfile::tempdir().unwrap();
            for name in ["adb.exe", "ldconsole.exe"] {
                std::fs::write(data.path().join(name), []).unwrap();
            }
            let app =
                Application::with_transport(data.path().to_owned(), root(), transport.clone())
                    .await
                    .unwrap();
            app.save_settings(Settings {
                device: Some(SelectedDevice {
                    folder: data.path().to_owned(),
                    index: 0,
                }),
                ..Default::default()
            })
            .await
            .unwrap();
            assert!(app.scan_army(source).await.is_err());
            assert!(!app.snapshot().army_complete);
            assert!(app.snapshot().army.is_empty());
            assert!(transport.before.taps.lock().unwrap().is_empty());
            assert!(transport.after.taps.lock().unwrap().is_empty());
            assert_eq!(
                transport.captures.load(Ordering::SeqCst),
                if source == Screen::BuilderScout { 2 } else { 1 }
            );
            app.shutdown().await.unwrap();
        }
    }

    #[derive(Clone)]
    struct LowMemoryDevice {
        base: Fake,
        probes: Arc<AtomicUsize>,
    }

    #[tokio::test]
    async fn plan_preview_requires_this_stages_complete_army_and_never_saves_or_taps() {
        use crate::vision::army::recipe::{Unit, UnitKind};
        let (base, _) = Fake::new("english-home.png");
        let data = tempfile::tempdir().unwrap();
        let mut app = Application::with_transport(data.path().to_owned(), root(), base.clone())
            .await
            .unwrap();
        let original = app.snapshot();
        let (observed, receiver) = watch::channel(original.clone());
        app.snapshot = receiver;
        assert!(app.preview_deployment(Screen::HomeScout).is_err());
        observed.send_modify(|s| {
            s.army_complete = true;
            s.army_screen = Some(Screen::HomeScout);
            s.army = vec![Unit {
                supports_ability: false,
                kind: UnitKind::Troop,
                slot: 1,
                count: Some(15),
            }];
        });
        let recipe = app.preview_deployment(Screen::HomeScout).unwrap();
        assert_eq!(recipe.steps.iter().map(|s| s.count).sum::<u32>(), 15);
        for screen in [Screen::Home, Screen::BuilderScout, Screen::BuilderStage2] {
            assert!(app.preview_deployment(screen).is_err());
        }
        observed.send_modify(|s| s.army_scanning = true);
        assert!(app.preview_deployment(Screen::HomeScout).is_err());
        observed.send_modify(|s| {
            s.army_scanning = false;
            s.army_complete = false;
        });
        assert!(app.preview_deployment(Screen::HomeScout).is_err());
        assert_eq!(app.snapshot().settings, original.settings);
        assert!(app.history(None).await.unwrap().is_empty());
        assert!(base.taps.lock().unwrap().is_empty());
        app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn army_observations_belong_to_the_selected_device() {
        let (base, _) = Fake::new("english-home.png");
        let data = tempfile::tempdir().unwrap();
        let app = Application::with_transport(data.path().to_owned(), root(), base.clone())
            .await
            .unwrap();
        let mut snapshot = app.snapshot();
        snapshot.settings.device = Some(SelectedDevice {
            folder: data.path().to_owned(),
            index: 0,
        });
        snapshot.army = vec![crate::vision::army::recipe::Unit {
            supports_ability: false,
            kind: crate::vision::army::recipe::UnitKind::Troop,
            slot: 1,
            count: Some(15),
        }];
        snapshot.army_complete = true;
        snapshot.army_screen = Some(Screen::HomeScout);
        snapshot.army_observed_at_ms = Some(1000);
        let mut settings = snapshot.settings.clone();
        settings.farming.attacks = 10;
        snapshot.set_settings(settings.clone());
        assert!(snapshot.army_complete);
        assert_eq!(snapshot.army.len(), 1);
        for folder_changed in [false, true] {
            let mut changed = snapshot.clone();
            let mut settings = changed.settings.clone();
            let device = settings.device.as_mut().unwrap();
            if folder_changed {
                device.folder = data.path().join("another-installation");
            } else {
                device.index = 1;
            }
            changed.set_settings(settings);
            assert!(changed.army.is_empty());
            assert!(!changed.army_complete);
            assert_eq!(changed.army_screen, None);
            assert_eq!(changed.army_observed_at_ms, None);
            assert_eq!(changed.settings.farming, snapshot.settings.farming);
        }
        assert!(base.taps.lock().unwrap().is_empty());
        app.shutdown().await.unwrap();
    }
    impl Transport for LowMemoryDevice {
        async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
            if command
                .program
                .file_name()
                .is_some_and(|name| name == "powershell.exe")
            {
                self.probes.fetch_add(1, Ordering::SeqCst);
                return Ok(b"512000".to_vec());
            }
            self.base.run(command, cancel).await
        }
    }

    #[tokio::test]
    async fn memory_checks_require_live_unpaused_between_attack_work_and_elapsed_cooldown() {
        let (base, _) = Fake::new("english-home.png");
        for (mode, paused, due, expected) in [
            (SessionMode::Observe, false, true, false),
            (SessionMode::Farming, true, true, false),
            (SessionMode::Farming, false, false, false),
            (SessionMode::Farming, false, true, true),
        ] {
            let data = tempfile::tempdir().unwrap();
            for name in ["adb.exe", "ldconsole.exe"] {
                std::fs::write(data.path().join(name), []).unwrap();
            }
            let (snapshot, _) = watch::channel(Snapshot {
                army: vec![],
                army_complete: false,
                army_screen: None,
                army_observed_at_ms: None,
                army_scanning: false,
                revision: 0,
                settings: Settings::default(),
                session: None,
                detection: Detection::Unknown,
                captured_at_ms: None,
                frame_sequence: None,
                resources: vec![],
                error: None,
                progress: None,
                statistics: vec![],
                default_layouts: BTreeMap::new(),
            });
            let fake = LowMemoryDevice {
                base: base.clone(),
                probes: Arc::default(),
            };
            let mut worker = Worker::load(
                data.path(),
                &root(),
                fake.clone(),
                snapshot,
                Arc::new(Mutex::new(None)),
                CancellationToken::new(),
            )
            .unwrap();
            worker
                .store
                .save_settings(&Settings {
                    device: Some(SelectedDevice {
                        folder: data.path().to_owned(),
                        index: 0,
                    }),
                    ..Settings::default()
                })
                .unwrap();
            worker.start(mode, CancellationToken::new()).unwrap();
            worker.pause(paused).unwrap();
            worker.running.as_mut().unwrap().next_memory_check =
                Instant::now() + Duration::from_secs(if due { 0 } else { 1800 });
            let result = worker.observe().await;
            assert_eq!(
                matches!(result, Err(Error::LowMemory)),
                expected,
                "{mode:?} paused={paused} due={due}: {result:?}"
            );
            if !expected {
                result.unwrap();
            }
            assert_eq!(fake.probes.load(Ordering::SeqCst), usize::from(expected));
            assert!(fake.base.taps.lock().unwrap().is_empty());
            worker.finish(SessionState::Stopped).unwrap();
        }
    }

    #[test]
    fn only_memory_exhaustion_is_a_recoverable_opencv_error() {
        assert!(recoverable(&Error::LowMemory));
        assert!(recoverable(&Error::OpenCv(opencv::Error::new(
            opencv::core::StsNoMem,
            "allocation failed"
        ))));
        assert!(!recoverable(&Error::OpenCv(opencv::Error::new(
            opencv::core::StsBadArg,
            "bad input"
        ))));
        assert!(!recoverable(&Error::Display));
        assert!(!recoverable(&Error::Cancelled));
    }
}
