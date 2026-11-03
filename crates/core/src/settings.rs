use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result, calibration::Resolution, device::DeviceConfig};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub device: Option<SelectedDevice>,
    #[serde(default)]
    pub farming: crate::gameplay::Options,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedDevice {
    pub folder: PathBuf,
    pub index: u8,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SettingsUpdate {
    Device(Option<SelectedDevice>),
    Farming(crate::gameplay::Options),
    Layout {
        screen: crate::calibration::Screen,
        layout: Option<crate::vision::army::Layout>,
    },
}

impl Settings {
    pub fn apply(&mut self, update: SettingsUpdate) -> Result<()> {
        match update {
            SettingsUpdate::Device(device) => {
                if let Some(device) = &device
                    && (!device.folder.join("adb.exe").is_file()
                        || !device.folder.join("ldconsole.exe").is_file())
                {
                    return Err(Error::Device(
                        "Choose the LDPlayer folder containing adb.exe and ldconsole.exe".into(),
                    ));
                }
                self.device = device;
            }
            SettingsUpdate::Farming(mut options) => {
                options.layouts = self.farming.layouts.clone();
                self.farming = options;
            }
            SettingsUpdate::Layout { screen, layout } => {
                if !matches!(
                    screen,
                    crate::calibration::Screen::HomeScout
                        | crate::calibration::Screen::BuilderScout
                        | crate::calibration::Screen::BuilderStage2
                ) {
                    return Err(Error::Calibration("Not a deployment screen".into()));
                }
                if let Some(layout) = layout {
                    self.farming.layouts.insert(screen, layout);
                } else {
                    self.farming.layouts.remove(&screen);
                }
            }
        }
        self.validate()
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(device) = &self.device
            && (!device.folder.is_absolute() || device.index > 99)
        {
            return Err(Error::Device(
                "Choose an absolute LDPlayer folder and instance 0–99".into(),
            ));
        }
        self.farming.validate(Resolution(1280, 720))
    }
}

impl SelectedDevice {
    pub fn config(&self, resolution: Resolution) -> DeviceConfig {
        DeviceConfig {
            adb: self.folder.join("adb.exe"),
            serial: format!("emulator-{}", 5554 + 2 * u16::from(self.index)),
            instance_index: self.index,
            resolution,
        }
    }
}
