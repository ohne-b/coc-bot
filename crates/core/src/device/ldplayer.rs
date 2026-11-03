use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::{
    InputMode,
    process::{Invocation, Process, Transport},
    valid_serial, validate_program,
};
use crate::{Error, Result, calibration::Resolution};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Instance {
    pub index: u8,
    pub name: String,
    pub running: bool,
    pub serial: String,
}

pub fn parse_instances(text: &str) -> Vec<Instance> {
    let mut instances = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<_> = line.trim().split(',').collect();
        if fields.len() < 7 || !matches!(fields[4], "0" | "1") {
            continue;
        }
        let Ok(index) = fields[0].parse::<u8>() else {
            continue;
        };
        if index > 99 {
            continue;
        }
        instances.entry(index).or_insert_with(|| Instance {
            index,
            name: fields[1].into(),
            running: fields[4] == "1",
            serial: format!("emulator-{}", 5554 + 2 * u16::from(index)),
        });
    }
    instances.into_values().collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Connected,
    Offline,
    Unauthorized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    pub serial: String,
    pub connection: Connection,
}

pub fn parse_devices(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            if !valid_serial(serial) {
                return None;
            }
            let connection = match fields.next()? {
                "device" => Connection::Connected,
                "offline" => Connection::Offline,
                "unauthorized" => Connection::Unauthorized,
                _ => return None,
            };
            Some(Device {
                serial: serial.into(),
                connection,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Display {
    pub resolution: Resolution,
    pub dpi: u32,
}

pub fn parse_display(size: &str, density: &str) -> Result<Display> {
    let value = |text: &str, label: &str| -> Result<String> {
        let physical = format!("Physical {label}: ");
        let overridden = format!("Override {label}: ");
        text.lines()
            .find_map(|line| line.trim().strip_prefix(&overridden))
            .or_else(|| {
                text.lines()
                    .find_map(|line| line.trim().strip_prefix(&physical))
            })
            .map(str::to_owned)
            .ok_or_else(|| Error::Device(format!("Emulator {label} could not be read")))
    };
    let size = value(size, "size")?;
    let (width, height) = size
        .split_once('x')
        .ok_or_else(|| Error::Device("Invalid emulator dimensions".into()))?;
    let number = |text: &str| {
        text.parse::<u32>()
            .map_err(|_| Error::Device("Invalid emulator display value".into()))
    };
    let resolution = Resolution(number(width)?, number(height)?);
    resolution.validate()?;
    let dpi = number(&value(density, "density")?)?;
    if !(72..=1000).contains(&dpi) {
        return Err(Error::Device("Invalid emulator DPI".into()));
    }
    Ok(Display { resolution, dpi })
}

pub struct LdPlayer<T = Process> {
    console: PathBuf,
    index: u8,
    mode: InputMode,
    transport: T,
}

impl<T: Transport> LdPlayer<T> {
    pub fn new(console: PathBuf, index: u8, mode: InputMode, transport: T) -> Result<Self> {
        validate_program(&console, "ldconsole.exe")?;
        if index > 99 {
            return Err(Error::Device(
                "LDPlayer index must be between 0 and 99".into(),
            ));
        }
        Ok(Self {
            console,
            index,
            mode,
            transport,
        })
    }

    pub async fn instances(&self, cancel: &CancellationToken) -> Result<Vec<Instance>> {
        let output = self
            .transport
            .run(
                Invocation {
                    program: self.console.clone(),
                    args: vec!["list2".into()],
                    input: vec![],
                    output_limit: 64 * 1024,
                    timeout: Duration::from_secs(15),
                },
                cancel,
            )
            .await?;
        Ok(parse_instances(&String::from_utf8_lossy(&output)))
    }

    pub async fn running(&self, cancel: &CancellationToken) -> Result<bool> {
        Ok(self
            .instances(cancel)
            .await?
            .iter()
            .any(|instance| instance.index == self.index && instance.running))
    }

    pub async fn launch(&mut self, cancel: &CancellationToken) -> Result<()> {
        self.change_state("launch", cancel).await
    }
    pub async fn quit(&mut self, cancel: &CancellationToken) -> Result<()> {
        self.change_state("quit", cancel).await
    }

    async fn change_state(&self, command: &str, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if self.mode != InputMode::Live {
            return Err(Error::InputDisabled);
        }
        self.transport
            .run(
                Invocation {
                    program: self.console.clone(),
                    args: vec![
                        command.into(),
                        "--index".into(),
                        self.index.to_string().into(),
                    ],
                    input: vec![],
                    output_limit: 64 * 1024,
                    timeout: Duration::from_secs(60),
                },
                cancel,
            )
            .await?;
        Ok(())
    }
}

/// Shallow searches only; never walk emulator disk images or arbitrary user directories.
pub async fn installed_folders<T: Transport>(
    configured: Option<PathBuf>,
    roots: Vec<PathBuf>,
    transport: &T,
    cancel: &CancellationToken,
) -> Result<Vec<PathBuf>> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let mut candidates = configured.into_iter().collect::<Vec<_>>();
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let program =
            PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let args = [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding = [Text.UTF8Encoding]::new(); Get-Process dnplayer,dnmultiplayer -ErrorAction SilentlyContinue | ForEach-Object { $_.Path }",
        ];
        let result = transport
            .run(
                Invocation {
                    program,
                    args: args.map(Into::into).to_vec(),
                    input: vec![],
                    output_limit: 64 * 1024,
                    timeout: Duration::from_secs(15),
                },
                cancel,
            )
            .await;
        match result {
            Ok(output) => candidates.extend(
                String::from_utf8_lossy(&output)
                    .lines()
                    .filter_map(|line| Path::new(line.trim()).parent().map(Path::to_path_buf)),
            ),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {} // Installed-folder search still works when process enumeration is unavailable.
        }
    }
    let worker_cancel = cancel.clone();
    let folders = tokio::task::spawn_blocking(move || {
        for root in roots.into_iter().take(26) {
            if worker_cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            for prefix in [
                "",
                "LDPlayer",
                "Program Files",
                "Program Files (x86)",
                "XuanZhi",
            ] {
                if worker_cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let Ok(entries) = std::fs::read_dir(root.join(prefix)) else {
                    continue;
                };
                for entry in entries.take(1024).flatten() {
                    if worker_cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    if entry
                        .file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with("ldplayer")
                    {
                        candidates.push(entry.path());
                    }
                }
            }
        }
        let mut folders = Vec::new();
        for path in candidates {
            if worker_cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if path.join("adb.exe").is_file()
                && path.join("ldconsole.exe").is_file()
                && let Ok(path) = path.canonicalize()
                && !folders.contains(&path)
            {
                folders.push(path);
            }
        }
        Ok(folders)
    })
    .await??;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(folders)
}

impl<T: Transport> super::Adb<T> {
    pub async fn verify(&mut self, cancel: &CancellationToken) -> Result<Display> {
        self.ready = false;
        if !self.devices(cancel).await?.iter().any(|device| {
            device.serial == self.config.serial && device.connection == Connection::Connected
        }) {
            return Err(Error::Device(
                "The selected emulator is disconnected or unauthorized".into(),
            ));
        }
        if !self.booted(cancel).await? {
            return Err(Error::Device(
                "The selected emulator is still starting".into(),
            ));
        }
        let display = self.display(cancel).await?;
        if display.resolution != self.config.resolution || display.dpi != 240 {
            return Err(Error::Device(
                "Set the emulator to the calibrated resolution and 240 DPI".into(),
            ));
        }
        self.ready = true;
        Ok(display)
    }

    pub async fn devices(&self, cancel: &CancellationToken) -> Result<Vec<Device>> {
        let output = self
            .transport
            .run(
                Invocation {
                    program: self.config.adb.clone(),
                    args: vec!["devices".into()],
                    input: vec![],
                    output_limit: 64 * 1024,
                    timeout: Duration::from_secs(15),
                },
                cancel,
            )
            .await?;
        Ok(parse_devices(&String::from_utf8_lossy(&output)))
    }

    pub async fn display(&self, cancel: &CancellationToken) -> Result<Display> {
        let size = self.command(&["shell", "wm", "size"], 4096, cancel).await?;
        let dpi = self
            .command(&["shell", "wm", "density"], 4096, cancel)
            .await?;
        parse_display(
            &String::from_utf8_lossy(&size),
            &String::from_utf8_lossy(&dpi),
        )
    }
}
