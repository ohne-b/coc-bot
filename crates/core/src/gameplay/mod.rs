mod engine;
mod options;
pub mod profile;
pub mod results;
mod wall_run;
pub mod walls;
pub use engine::{Engine, Outcome, Progress};
pub use options::{Options, Strategy};

pub struct GameVision<'a> {
    pub profile: &'a profile::Profile,
    pub digits: &'a crate::vision::Digits,
    pub detector: std::sync::Arc<crate::vision::Detector>,
}
pub struct FrameView<'a> {
    pub image: &'a opencv::core::Mat,
    pub detection: &'a crate::vision::Detection,
    pub elapsed_ms: u64,
    pub captured_at_ms: i64,
}
