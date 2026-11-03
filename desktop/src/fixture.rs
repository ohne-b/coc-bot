use std::{path::PathBuf, sync::Arc};

use coc_bot_core::{
    Error, Result,
    application::Application,
    device::process::{Invocation, Transport},
    settings::{SelectedDevice, Settings},
    vision::read_image,
};
use opencv::{core::Mat, imgproc, prelude::*};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Fixture(Arc<Vec<u8>>);

impl Transport for Fixture {
    async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let args = command
            .args
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(match args.as_str() {
            "devices" => b"List of devices attached\nemulator-5554 device\n".to_vec(),
            "-s emulator-5554 shell getprop sys.boot_completed" => b"1".to_vec(),
            "-s emulator-5554 shell wm size" => b"Physical size: 1280x720".to_vec(),
            "-s emulator-5554 shell wm density" => b"Physical density: 240".to_vec(),
            "-s emulator-5554 exec-out screencap" => self.0.as_ref().clone(),
            "list2" => b"0,Fixture,0,0,1,0,0\n".to_vec(),
            _ => {
                return Err(Error::Device(
                    "Device command disabled in fixture build".into(),
                ));
            }
        })
    }
}

pub async fn open(assets: PathBuf) -> Result<(Application, tempfile::TempDir)> {
    let data = tempfile::tempdir()?;
    for name in ["adb.exe", "ldconsole.exe"] {
        std::fs::write(data.path().join(name), [])?;
    }
    let image = read_image(&assets.join("fixtures/english-home.png"))?;
    let mut rgba = Mat::default();
    imgproc::cvt_color_def(&image, &mut rgba, imgproc::COLOR_BGR2RGBA)?;
    let bytes = [1280_u32, 720, 1]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .chain(rgba.data_bytes()?.iter().copied())
        .collect();
    let app = Application::with_transport(data.path().to_owned(), assets, Fixture(Arc::new(bytes)))
        .await?;
    app.save_settings(Settings {
        device: Some(SelectedDevice {
            folder: data.path().to_owned(),
            index: 0,
        }),
        ..Settings::default()
    })
    .await?;
    app.start(coc_bot_core::telemetry::SessionMode::Observe)
        .await?;
    Ok((app, data))
}
