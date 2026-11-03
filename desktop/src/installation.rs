use std::path::Path;

use coc_bot_core::{Result, application::Application};

/// Check bundled calibration, OpenCV and SQLite without opening a window or device.
pub fn check(assets: &Path) -> Result<()> {
    let data = tempfile::tempdir()?;
    tokio::runtime::Runtime::new()?.block_on(async {
        let app = Application::open(data.path().to_owned(), assets.to_owned()).await?;
        let result = app.history(None).await;
        app.shutdown().await?;
        result.map(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_assets_are_required_and_checks_leave_them_unchanged() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
        assert!(!assets.join("history.sqlite3").exists());
        check(&assets).unwrap();
        assert!(!assets.join("history.sqlite3").exists());
        let empty = tempfile::tempdir().unwrap();
        assert!(check(empty.path()).is_err());
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
    }
}
