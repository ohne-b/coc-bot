use crate::{
    Error, Result,
    calibration::Resolution,
    device::{
        Adb, DeviceConfig, InputMode,
        process::{Invocation, Transport},
    },
    vision::read_image,
};
use opencv::{core::Mat, imgproc, prelude::*};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}
#[derive(Clone)]
pub struct Fake {
    frame: Arc<Vec<u8>>,
    pub taps: Arc<Mutex<Vec<String>>>,
    pub fail_after_tap: bool,
}
impl Fake {
    pub fn new(file: &str) -> (Self, Mat) {
        let image = read_image(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(file),
        )
        .unwrap();
        (Self::from_image(&image), image)
    }
    pub fn from_image(image: &Mat) -> Self {
        let mut rgba = Mat::default();
        imgproc::cvt_color_def(&image, &mut rgba, imgproc::COLOR_BGR2RGBA).unwrap();
        let frame = [1280_u32, 720, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .chain(rgba.data_bytes().unwrap().iter().copied())
            .collect();
        Self {
            frame: Arc::new(frame),
            taps: Arc::default(),
            fail_after_tap: false,
        }
    }
    pub fn adb(&self, directory: &Path) -> Adb<Self> {
        std::fs::write(directory.join("adb.exe"), []).unwrap();
        Adb::new(
            DeviceConfig {
                adb: directory.join("adb.exe"),
                serial: "emulator-5554".into(),
                instance_index: 0,
                resolution: Resolution(1280, 720),
            },
            InputMode::Live,
            self.clone(),
        )
        .unwrap()
    }
}
impl Transport for Fake {
    async fn run(&self, command: Invocation, cancel: &CancellationToken) -> Result<Vec<u8>> {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let args = command
            .args
            .iter()
            .map(|v| v.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        if args.contains("shell input tap")
            || args.contains("shell input swipe")
            || args.contains("/dev/input/event")
        {
            self.taps.lock().unwrap().push(args);
            return if self.fail_after_tap {
                Err(Error::Timeout)
            } else {
                Ok(vec![])
            };
        }
        Ok(match args.as_str() {
            "devices" => b"List of devices attached\nemulator-5554 device\n".to_vec(),
            "-s emulator-5554 shell getprop sys.boot_completed" => b"1".to_vec(),
            "-s emulator-5554 shell wm size" => b"Physical size: 1280x720".to_vec(),
            "-s emulator-5554 shell wm density" => b"Physical density: 240".to_vec(),
            "-s emulator-5554 shell uname -m" => b"x86_64".to_vec(),
            "-s emulator-5554 shell getevent -pl" => b"add device 1: /dev/input/event4\n name: ldtouch\n ABS (0003): ABS_MT_SLOT : value 0, min 0, max 9\n ABS_MT_POSITION_X : value 0, min 0, max 1279\n ABS_MT_POSITION_Y : value 0, min 0, max 719\n".to_vec(),
            "-s emulator-5554 exec-out screencap" => self.frame.as_ref().clone(),
            _ => panic!("Unexpected device command {args}"),
        })
    }
}
