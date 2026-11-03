pub mod calibration;
pub mod vision;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Calibration(String),
    #[error("Screenshot dimensions do not match the calibration")]
    Resolution,
    #[error("Screenshot data is incomplete or uses an unsupported format")]
    Frame,
    #[error("The value could not be read reliably")]
    Unreadable,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    OpenCv(#[from] opencv::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
