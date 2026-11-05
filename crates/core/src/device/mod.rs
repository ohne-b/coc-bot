use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use opencv::core::Mat;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result,
    calibration::{Resolution, Screen},
    vision::{Detection, Detector, decode_frame},
};

pub mod ldplayer;
pub mod process;
mod touch;
pub mod update;
use process::{Invocation, Process, Transport};
pub use touch::{Gesture, TouchDevice};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FRAME_AGE: Duration = Duration::from_secs(2);
const GAME: &str = "com.supercell.clashofclans";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Point(pub u32, pub u32);

impl Point {
    pub fn validate(self, resolution: Resolution) -> Result<()> {
        if self.0 >= resolution.0 || self.1 >= resolution.1 {
            return Err(Error::Device("Input point is outside the screen".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Observe,
    Live,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceConfig {
    pub adb: PathBuf,
    pub serial: String,
    pub resolution: Resolution,
    pub instance_index: u8,
}

impl DeviceConfig {
    pub fn validate(&self) -> Result<()> {
        self.resolution.validate()?;
        validate_program(&self.adb, "adb.exe")?;
        if self.instance_index > 99
            || !valid_serial(&self.serial)
            || ![
                format!("emulator-{}", 5554 + 2 * u16::from(self.instance_index)),
                format!("127.0.0.1:{}", 5555 + 2 * u16::from(self.instance_index)),
            ]
            .contains(&self.serial)
        {
            return Err(Error::Device(
                "ADB serial must belong to the selected LDPlayer instance".into(),
            ));
        }
        Ok(())
    }
}

pub struct CapturedFrame {
    pub image: Mat,
    pub captured_at: Instant,
}

pub struct Adb<T = Process> {
    config: DeviceConfig,
    mode: InputMode,
    transport: T,
    ready: bool,
}

impl<T: Transport> Adb<T> {
    pub fn new(config: DeviceConfig, mode: InputMode, transport: T) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            mode,
            transport,
            ready: false,
        })
    }

    pub fn config(&self) -> &DeviceConfig {
        &self.config
    }

    async fn command(
        &self,
        args: &[&str],
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let mut arguments = vec![OsString::from("-s"), self.config.serial.clone().into()];
        arguments.extend(args.iter().map(OsString::from));
        self.transport
            .run(
                Invocation {
                    program: self.config.adb.clone(),
                    args: arguments,
                    input: vec![],
                    acknowledge_input: false,
                    output_limit: limit,
                    timeout: COMMAND_TIMEOUT,
                },
                cancel,
            )
            .await
    }

    pub async fn capture(&mut self, cancel: &CancellationToken) -> Result<CapturedFrame> {
        let resolution = self.config.resolution;
        let limit = resolution.0 as usize * resolution.1 as usize * 4 + 16;
        for attempt in 0..3 {
            let captured_at = Instant::now();
            let result = match self
                .command(&["exec-out", "screencap"], limit, cancel)
                .await
            {
                Ok(bytes) => {
                    tokio::task::spawn_blocking(move || decode_frame(&bytes, resolution)).await?
                }
                Err(error) => Err(error),
            };
            if cancel.is_cancelled() {
                self.ready = false;
                return Err(Error::Cancelled);
            }
            match result {
                Ok(image) => return Ok(CapturedFrame { image, captured_at }),
                Err(error)
                    if attempt == 2
                        || matches!(
                            error,
                            Error::Cancelled | Error::OutputLimit | Error::Resolution
                        ) =>
                {
                    self.ready = false;
                    return Err(error);
                }
                Err(_) => {
                    self.ready = false;
                }
            }
        }
        unreachable!("capture attempts are bounded")
    }

    fn require_live(&self, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if self.mode != InputMode::Live {
            return Err(Error::InputDisabled);
        }
        Ok(())
    }

    async fn guard(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        cancel: &CancellationToken,
    ) -> Result<CapturedFrame> {
        self.require_live(cancel)?;
        if expected.is_empty() {
            return Err(Error::InputGuard);
        }
        if !self.ready {
            self.verify(cancel).await?;
        }
        let frame = self.capture(cancel).await?;
        let captured_at = frame.captured_at;
        let (image, detection) = tokio::task::spawn_blocking(move || {
            let detection = detector.detect(&frame.image)?;
            Ok::<_, Error>((frame.image, detection))
        })
        .await??;
        if !self.ready
            || captured_at.elapsed() > MAX_FRAME_AGE
            || !matches!(detection,Detection::Known(screen) if expected.contains(&screen))
        {
            return Err(Error::InputGuard);
        }
        self.require_live(cancel)?;
        Ok(CapturedFrame { image, captured_at })
    }

    pub async fn tap(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        point: Point,
        cancel: &CancellationToken,
    ) -> Result<()> {
        point.validate(self.config.resolution)?;
        self.tap_checked(detector, expected, |_| Ok((point, ())), cancel)
            .await
    }

    /// Resolve moving targets and purchase guards against the exact frame authorizing input.
    pub async fn tap_checked<R>(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        resolve: impl FnOnce(&Mat) -> Result<(Point, R)>,
        cancel: &CancellationToken,
    ) -> Result<R> {
        let frame = self.guard(detector, expected, cancel).await?;
        let (point, value) = resolve(&frame.image)?;
        point.validate(self.config.resolution)?;
        if frame.captured_at.elapsed() > MAX_FRAME_AGE {
            return Err(Error::InputGuard);
        }
        self.require_live(cancel)?;
        let result = self
            .command(
                &[
                    "shell",
                    "input",
                    "tap",
                    &point.0.to_string(),
                    &point.1.to_string(),
                ],
                4096,
                cancel,
            )
            .await;
        if result.is_err() {
            self.ready = false;
        }
        result.map_err(dispatched_error).map(|_| value)
    }

    pub async fn swipe(
        &mut self,
        detector: Arc<Detector>,
        expected: &[Screen],
        start: Point,
        end: Point,
        cancel: &CancellationToken,
    ) -> Result<()> {
        start.validate(self.config.resolution)?;
        end.validate(self.config.resolution)?;
        self.guard(detector, expected, cancel).await?;
        let result = self
            .command(
                &[
                    "shell",
                    "input",
                    "swipe",
                    &start.0.to_string(),
                    &start.1.to_string(),
                    &end.0.to_string(),
                    &end.1.to_string(),
                    "300",
                ],
                4096,
                cancel,
            )
            .await;
        if result.is_err() {
            self.ready = false;
        }
        result.map(|_| ())
    }

    pub async fn booted(&self, cancel: &CancellationToken) -> Result<bool> {
        Ok(self
            .command(&["shell", "getprop", "sys.boot_completed"], 4096, cancel)
            .await?
            .trim_ascii()
            == b"1")
    }

    pub async fn game_in_front(&self, cancel: &CancellationToken) -> Result<bool> {
        let output = self
            .command(&["shell", "dumpsys", "window"], 1024 * 1024, cancel)
            .await?;
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .any(|line| line.contains("mCurrentFocus") && line.contains(&format!("{GAME}/"))))
    }

    pub async fn launch_game(&mut self, cancel: &CancellationToken) -> Result<()> {
        self.require_live(cancel)?;
        self.ready = false;
        self.command(
            &[
                "shell",
                "am",
                "start",
                "-n",
                &format!("{GAME}/com.supercell.titan.GameApp"),
            ],
            64 * 1024,
            cancel,
        )
        .await?;
        Ok(())
    }

    pub async fn stop_game(&mut self, cancel: &CancellationToken) -> Result<()> {
        self.require_live(cancel)?;
        self.ready = false;
        self.command(&["shell", "am", "force-stop", GAME], 4096, cancel)
            .await?;
        Ok(())
    }
}

fn dispatched_error(error: Error) -> Error {
    // Only checks before dispatch can prove that input was withheld.
    match error {
        Error::InputGuard => Error::Device("Input transport failed after dispatch".into()),
        other => other,
    }
}

fn valid_serial(serial: &str) -> bool {
    !serial.is_empty()
        && serial.len() <= 128
        && !serial.starts_with('-')
        && serial
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
}

fn validate_program(program: &Path, name: &str) -> Result<()> {
    if !program.is_absolute()
        || !program.is_file()
        || !program
            .file_name()
            .is_some_and(|file| file.to_string_lossy().eq_ignore_ascii_case(name))
    {
        return Err(Error::Device(format!("Select the full path to {name}")));
    }
    Ok(())
}
