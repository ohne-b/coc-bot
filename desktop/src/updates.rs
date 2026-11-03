use std::{sync::atomic::Ordering, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State, WebviewWindow};
use tauri_plugin_updater::Update;
#[cfg(not(feature = "desktop-fixture"))]
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::Desktop;

const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Installing,
    Failed,
}

#[derive(Clone, Serialize)]
pub struct Status {
    revision: u64,
    phase: Phase,
    current_version: &'static str,
    version: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    error: Option<&'static str>,
    restart_required: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: Phase::Idle,
            current_version: env!("CARGO_PKG_VERSION"),
            version: None,
            downloaded: 0,
            total: None,
            error: None,
            restart_required: false,
        }
    }
}

#[derive(Default)]
struct Pending {
    status: Status,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
    cancel: Option<CancellationToken>,
}

#[derive(Default)]
pub struct Updates {
    pending: Mutex<Pending>,
    pub tasks: TaskTracker,
    pub cancel: CancellationToken,
}

impl Updates {
    pub async fn shutdown(&self) {
        // Serialize tracker closure with command registration before waiting for owned work.
        let pending = self.pending.lock().await;
        self.cancel.cancel();
        self.tasks.close();
        drop(pending);
        self.tasks.wait().await;
    }
}

impl Pending {
    fn validate(&self, action: &Action) -> Result<(), String> {
        if matches!(
            self.status.phase,
            Phase::Checking | Phase::Downloading | Phase::Installing
        ) || self.status.restart_required
        {
            return Err("Updater is busy".to_string());
        }
        if matches!(action, Action::Download) && self.update.is_none()
            || matches!(action, Action::Install)
                && (self.status.phase != Phase::Ready
                    || self.bytes.is_none()
                    || self.update.is_none())
        {
            return Err("Invalid update action".to_string());
        }
        Ok(())
    }

    fn downloaded(
        &mut self,
        result: Option<tauri_plugin_updater::Result<Vec<u8>>>,
        oversized: bool,
        cancelled: bool,
    ) {
        self.bytes = None;
        match result {
            _ if cancelled && !oversized => self.status.phase = Phase::Available,
            Some(Ok(bytes)) if !oversized && !cancelled => {
                self.status.phase = Phase::Ready;
                self.status.downloaded = bytes.len() as u64;
                self.bytes = Some(bytes);
            }
            None if !oversized => self.status.phase = Phase::Available,
            _ => {
                self.status.phase = Phase::Failed;
                self.status.error = Some("download_failed");
            }
        }
        self.cancel = None;
    }

    fn install_failed(&mut self) {
        self.status.phase = Phase::Failed;
        self.status.error = Some("install_failed");
        self.status.restart_required = true;
        self.cancel = None;
    }

    fn publish(&mut self, app: &tauri::AppHandle) -> Status {
        self.status.revision += 1;
        let _ = app.emit_to("main", "desktop-update", &self.status);
        self.status.clone()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Check,
    Download,
    Cancel,
    Install,
}

#[tauri::command]
pub async fn desktop_update(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Desktop>,
    action: Option<Action>,
) -> Result<Status, String> {
    if window.label() != "main" {
        return Err("Unavailable window".into());
    }
    let Some(action) = action else {
        return Ok(state.updates.pending.lock().await.status.clone());
    };
    start(&app, action).await
}

pub async fn start(app: &tauri::AppHandle, action: Action) -> Result<Status, String> {
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    if matches!(action, Action::Cancel) {
        if let Some(cancel) = pending.cancel.as_ref() {
            cancel.cancel();
        }
        return Ok(pending.status.clone());
    }
    if state.closing.load(Ordering::SeqCst) {
        return Err("Updater is busy".into());
    }
    pending.validate(&action)?;
    if matches!(action, Action::Install)
        && (cfg!(feature = "desktop-fixture")
            || tauri::utils::platform::bundle_type()
                != Some(tauri::utils::config::BundleType::Nsis))
    {
        return Err("Updates can only be installed from the installed Windows application.".into());
    }
    pending.status.error = None;
    let cancel = state.updates.cancel.child_token();
    pending.cancel = Some(cancel.clone());
    let handle = app.clone();
    match action {
        Action::Check => {
            pending.status.phase = Phase::Checking;
            pending.status.version = None;
            pending.update = None;
            pending.bytes = None;
            state.updates.tasks.spawn(async move {
                check(handle, cancel).await;
            });
        }
        Action::Download => {
            let update = pending
                .update
                .clone()
                .ok_or("Invalid update action".to_string())?;
            pending.status.phase = Phase::Downloading;
            pending.status.downloaded = 0;
            pending.status.total = None;
            state.updates.tasks.spawn(async move {
                download(handle, update, cancel).await;
            });
        }
        Action::Install => {
            if state.closing.swap(true, Ordering::SeqCst) {
                return Err("Application is shutting down".into());
            }
            let update = pending
                .update
                .take()
                .ok_or("Invalid update action".to_string())?;
            let bytes = pending
                .bytes
                .take()
                .ok_or("Invalid update action".to_string())?;
            pending.status.phase = Phase::Installing;
            state.updates.tasks.spawn(async move {
                let state = handle.state::<Desktop>();
                if state.drain().await.is_err() {
                    let mut pending = state.updates.pending.lock().await;
                    pending.install_failed();
                    pending.publish(&handle);
                    state.closing.store(false, Ordering::SeqCst);
                    return;
                }
                // The Windows updater exits the process. Bot and database work is already drained.
                let result =
                    tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
                if matches!(result, Ok(Ok(()))) {
                    handle.restart();
                }
                let mut pending = state.updates.pending.lock().await;
                pending.install_failed();
                pending.publish(&handle);
                state.closing.store(false, Ordering::SeqCst);
            });
        }
        Action::Cancel => unreachable!(),
    }
    Ok(pending.publish(app))
}

async fn check(app: tauri::AppHandle, cancel: CancellationToken) {
    #[cfg(feature = "desktop-fixture")]
    let operation = async { Ok::<Option<Update>, tauri_plugin_updater::Error>(None) };
    #[cfg(not(feature = "desktop-fixture"))]
    let operation = async {
        let updater = app
            .updater_builder()
            .target("windows-x86_64-nsis")
            .build()?;
        updater.check().await
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(20), operation) => Some(result),
    };
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    match result {
        Some(Ok(Ok(Some(update)))) if valid_download(&update.download_url, &update.version) => {
            pending.status.version = Some(update.version.clone());
            pending.status.phase = Phase::Available;
            pending.update = Some(update);
        }
        Some(Ok(Ok(None))) => pending.status.phase = Phase::Current,
        None => pending.status.phase = Phase::Idle,
        _ => {
            pending.status.phase = Phase::Failed;
            pending.status.error = Some("check_failed");
        }
    }
    pending.cancel = None;
    pending.publish(&app);
}

fn valid_download(url: &tauri::Url, version: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path()
            == format!(
                "/ohne-b/coc-bot/releases/download/v{version}/coc-bot-{version}-windows-x64-setup.exe"
            )
}

async fn download(app: tauri::AppHandle, update: Update, cancel: CancellationToken) {
    let mut downloaded = 0u64;
    let mut progress = None;
    let mut oversized = false;
    let operation = update.download(
        |size, total| {
            downloaded = downloaded.saturating_add(size as u64);
            if downloaded > MAX_DOWNLOAD || total.is_some_and(|size| size > MAX_DOWNLOAD) {
                oversized = true;
                cancel.cancel();
                return;
            }
            let next = total
                .filter(|v| *v > 0)
                .map_or(downloaded / (1024 * 1024), |v| {
                    downloaded.saturating_mul(100) / v
                });
            if progress != Some(next) {
                progress = Some(next);
                // The callback is synchronous; this lock never crosses an await in the downloader.
                if let Ok(mut pending) = app.state::<Desktop>().updates.pending.try_lock() {
                    pending.status.downloaded = downloaded;
                    pending.status.total = total;
                    pending.publish(&app);
                }
            }
        },
        || {},
    );
    let result = download_result(operation, &cancel).await;
    let state = app.state::<Desktop>();
    let mut pending = state.updates.pending.lock().await;
    pending.downloaded(result, oversized, cancel.is_cancelled());
    pending.publish(&app);
}

async fn download_result(
    operation: impl std::future::Future<Output = tauri_plugin_updater::Result<Vec<u8>>>,
    cancel: &CancellationToken,
) -> Option<tauri_plugin_updater::Result<Vec<u8>>> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(600), operation) => Some(
            result.unwrap_or_else(|_| Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into()))
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_updater::UpdaterExt;

    #[test]
    fn native_verifier_rejects_tampering_wrong_versions_and_unsigned_versions() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/update.json")).unwrap();
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({"pubkey": fixture["pubkey"], "requireSignedVersion": true}),
        );
        let app = tauri::test::mock_builder()
            .plugin(
                tauri_plugin_updater::Builder::new()
                    .pubkey(fixture["pubkey"].as_str().unwrap())
                    .build(),
            )
            .build(context)
            .unwrap();
        let updater = app
            .updater_builder()
            .target("windows-x86_64-nsis")
            .endpoints(vec!["https://example.invalid/update".parse().unwrap()])
            .unwrap()
            .configure_client(|_| panic!("signature tests must remain offline"))
            .build()
            .unwrap();
        let manifest = |version: &str, signature: &serde_json::Value| {
            serde_json::json!({
                "version": version,
                "platforms": { "windows-x86_64-nsis": {
                    "url": format!("https://github.com/ohne-b/coc-bot/releases/download/v{version}/coc-bot-{version}-windows-x64-setup.exe"),
                    "signature": signature
                }}
            })
        };
        let payload = fixture["payload"].as_str().unwrap().as_bytes();
        let update = updater
            .restore_update(manifest("2.0.0", &fixture["signature"]))
            .unwrap()
            .unwrap();
        update.verify(payload).unwrap();
        assert!(valid_download(&update.download_url, &update.version));
        assert!(update.verify(b"tampered").is_err());
        assert!(
            updater
                .restore_update(manifest("3.0.0", &fixture["signature"]))
                .unwrap()
                .unwrap()
                .verify(payload)
                .is_err()
        );
        assert!(
            updater
                .restore_update(manifest("2.0.0", &fixture["unversioned_signature"]))
                .unwrap()
                .unwrap()
                .verify(payload)
                .is_err()
        );
        assert!(
            updater
                .restore_update(manifest("0.0.0", &fixture["signature"]))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn only_verified_downloads_can_install_and_failed_install_requires_restart() {
        let mut pending = Pending::default();
        assert!(pending.validate(&Action::Download).is_err());
        assert!(pending.validate(&Action::Install).is_err());
        for phase in [Phase::Checking, Phase::Downloading, Phase::Installing] {
            pending.status.phase = phase;
            for action in [Action::Check, Action::Download, Action::Install] {
                assert!(pending.validate(&action).is_err());
            }
        }
        for error in [
            tauri_plugin_updater::Error::MissingSignedVersion,
            tauri_plugin_updater::Error::Network("interrupted".into()),
        ] {
            pending.downloaded(Some(Err(error)), false, false);
            assert_eq!(pending.status.phase, Phase::Failed);
            assert_eq!(pending.status.error, Some("download_failed"));
            assert!(pending.bytes.is_none());
            assert!(pending.validate(&Action::Install).is_err());
            assert!(pending.validate(&Action::Check).is_ok());
        }
        pending.downloaded(None, false, true);
        assert_eq!(pending.status.phase, Phase::Available);
        assert!(pending.bytes.is_none());
        pending.downloaded(Some(Ok(vec![1])), false, true);
        assert_eq!(pending.status.phase, Phase::Available);
        assert!(pending.bytes.is_none());
        pending.downloaded(Some(Ok(vec![1])), true, true);
        assert_eq!(pending.status.phase, Phase::Failed);
        assert!(pending.bytes.is_none());
        pending.downloaded(Some(Ok(vec![1])), false, false);
        assert_eq!(pending.status.phase, Phase::Ready);
        assert_eq!(pending.bytes.take(), Some(vec![1]));
        pending.install_failed();
        assert_eq!(pending.status.error, Some("install_failed"));
        assert!(pending.status.restart_required);
        for action in [Action::Check, Action::Download, Action::Install] {
            assert!(pending.validate(&action).is_err());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_download_times_out_and_cancellation_does_not_become_an_error() {
        let cancel = CancellationToken::new();
        let start = tokio::time::Instant::now();
        let stalled = download_result(std::future::pending(), &cancel).await;
        assert!(
            matches!(stalled, Some(Err(tauri_plugin_updater::Error::Io(error)))
            if error.kind() == std::io::ErrorKind::TimedOut)
        );
        assert_eq!(start.elapsed(), Duration::from_secs(600));
        assert_eq!(
            download_result(async { Ok(vec![1]) }, &cancel)
                .await
                .unwrap()
                .unwrap(),
            vec![1]
        );
        cancel.cancel();
        assert!(
            download_result(std::future::pending(), &cancel)
                .await
                .is_none()
        );
    }

    #[test]
    fn installer_links_are_confined_to_the_announced_release() {
        let good = "https://github.com/ohne-b/coc-bot/releases/download/v1.7.0/coc-bot-1.7.0-windows-x64-setup.exe";
        assert!(valid_download(&tauri::Url::parse(good).unwrap(), "1.7.0"));
        for bad in [
            good.replace("github.com", "example.com"),
            good.replace("https:", "http:"),
            good.replace("v1.7.0", "v1.6.0"),
            format!("{good}?redirect=elsewhere"),
            good.replace("github.com", "user@github.com"),
        ] {
            assert!(!valid_download(&tauri::Url::parse(&bad).unwrap(), "1.7.0"));
        }
    }
}
