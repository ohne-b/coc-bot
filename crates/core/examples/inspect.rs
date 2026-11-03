use std::path::Path;

use coc_bot_core::{
    calibration::Calibration,
    vision::{Detector, read_image},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [profile_path, screenshot_path] = args.as_slice() else {
        return Err("Usage: inspect <calibration.json> <screenshot.png>".into());
    };
    let path = Path::new(profile_path);
    let profile = Calibration::load(path)?;
    let detector = Detector::load(&profile, path.parent().unwrap_or(Path::new(".")))?;
    let result = detector.detect(&read_image(Path::new(screenshot_path))?)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
