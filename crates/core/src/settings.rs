use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result, calibration::Resolution, device::DeviceConfig};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub device: Option<SelectedDevice>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedDevice {
    pub folder: PathBuf,
    pub index: u8,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if let Some(device) = &self.device
            && (!device.folder.is_absolute() || device.index > 99)
        {
            return Err(Error::Device(
                "Choose an absolute LDPlayer folder and instance 0–99".into(),
            ));
        }
        Ok(())
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
