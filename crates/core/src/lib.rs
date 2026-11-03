pub mod application;
pub mod calibration;
pub mod device;
pub mod gameplay;
pub mod settings;
pub mod storage;
pub mod telemetry;
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
    #[error("{0}")]
    History(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Device(String),
    #[error("{0}")]
    Gameplay(String),
    #[error("Operation cancelled")]
    Cancelled,
    #[error("Device command timed out")]
    Timeout,
    #[error("Device output exceeded its limit")]
    OutputLimit,
    #[error("Input is disabled in observation mode")]
    InputDisabled,
    #[error("The expected screen was not freshly recognized; no input was sent")]
    InputGuard,
    #[error("Background work failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    OpenCv(#[from] opencv::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
