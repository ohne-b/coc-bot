use super::{Adb, Point, process::Transport};
use crate::{Error, Result, calibration::Resolution};
use roxmltree::{Document, ParsingOptions};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

const STORE: &str = "com.android.vending";

pub fn update_button(xml: &str, resolution: Resolution) -> Option<Point> {
    if xml.len() > 2 * 1024 * 1024 {
        return None;
    }
    let start = xml.find("<hierarchy")?;
    let end = xml.rfind("</hierarchy>")? + 12;
    let doc = Document::parse_with_options(
        xml.get(start..end)?,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 10000,
            ..Default::default()
        },
    )
    .ok()?;
    if !doc.descendants().any(|node| {
        node.attribute("package") == Some(STORE)
            && ["text", "content-desc"].iter().any(|key| {
                node.attribute(*key)
                    .is_some_and(|value| value.contains("Clash of Clans"))
            })
    }) {
        return None;
    }
    let mut points = Vec::new();
    for node in doc.descendants().filter(|node| {
        node.attribute("package") == Some(STORE)
            && ["text", "content-desc"].iter().any(|key| {
                node.attribute(*key)
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("Update"))
            })
    }) {
        let button = node
            .ancestors()
            .find(|node| node.attribute("clickable") == Some("true"))?;
        if button.attribute("package") != Some(STORE) || button.attribute("enabled") != Some("true")
        {
            continue;
        }
        let bounds = button
            .attribute("bounds")?
            .strip_prefix('[')?
            .strip_suffix(']')?;
        let (left, right) = bounds.split_once("][")?;
        let (x, y) = left.split_once(',')?;
        let (right, bottom) = right.split_once(',')?;
        let (x, y, right, bottom) = (
            x.parse::<u32>().ok()?,
            y.parse::<u32>().ok()?,
            right.parse::<u32>().ok()?,
            bottom.parse::<u32>().ok()?,
        );
        if x >= right || y >= bottom || right > resolution.0 || bottom > resolution.1 {
            return None;
        }
        let point = Point(x + (right - x) / 2, y + (bottom - y) / 2);
        if !points.contains(&point) {
            points.push(point);
        }
    }
    match points.as_slice() {
        [point] => Some(*point),
        _ => None,
    }
}

pub fn installed_version(text: &str) -> Option<String> {
    let mut fields = Vec::new();
    for key in ["versionName=", "versionCode=", "lastUpdateTime="] {
        let line = text
            .lines()
            .find_map(|line| line.trim().strip_prefix(key))?;
        let value = if key == "lastUpdateTime=" {
            line.trim()
        } else {
            line.split_whitespace().next()?
        };
        if value.is_empty() || value.len() > 100 {
            return None;
        }
        fields.push(value);
    }
    Some(fields.join(" / "))
}
impl<T: Transport> Adb<T> {
    pub(crate) async fn game_version(&self, cancel: &CancellationToken) -> Result<Option<String>> {
        let data = self
            .command(
                &["shell", "dumpsys", "package", super::GAME],
                2 * 1024 * 1024,
                cancel,
            )
            .await?;
        Ok(installed_version(&String::from_utf8_lossy(&data)))
    }
    pub(crate) async fn press_store_update(&mut self, cancel: &CancellationToken) -> Result<bool> {
        self.require_live(cancel)?;
        self.verify(cancel).await?;
        let at = Instant::now();
        let xml = self
            .command(
                &["exec-out", "uiautomator", "dump", "/dev/tty"],
                2 * 1024 * 1024,
                cancel,
            )
            .await?;
        let Some(point) = update_button(&String::from_utf8_lossy(&xml), self.config.resolution)
        else {
            return Ok(false);
        };
        let focus = self
            .command(&["shell", "dumpsys", "window"], 1024 * 1024, cancel)
            .await?;
        if !String::from_utf8_lossy(&focus)
            .lines()
            .any(|line| line.contains("mCurrentFocus") && line.contains("com.android.vending/"))
            || at.elapsed() > std::time::Duration::from_secs(5)
        {
            return Err(Error::InputGuard);
        }
        self.require_live(cancel)?;
        self.command(
            &[
                "shell",
                "input",
                "tap",
                &point.0.to_string(),
                &point.1.to_string(),
            ],
            4096,
            cancel,
        )
        .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OFFER: &str =
        include_str!("../../../../legacy/python/fixtures/playstore-update-offer.xml");
    const PAGE: &str = include_str!("../../../../legacy/python/fixtures/playstore-game-page.xml");
    #[test]
    fn only_an_enabled_english_update_for_the_game_is_accepted() {
        let res = Resolution(1280, 720);
        assert_eq!(update_button(OFFER, res), Some(Point(829, 497)));
        assert_eq!(update_button(PAGE, res), Some(Point(466, 466)));
        for (from, to) in [
            ("Update", "Install"),
            ("Update", "Perbarui"),
            ("Update", "Cancel"),
            ("Clash of Clans", "Other Game"),
            ("com.android.vending", "other.package"),
            ("enabled=\"true\"", "enabled=\"false\""),
        ] {
            assert_eq!(update_button(&OFFER.replace(from, to), res), None);
        }
        assert_eq!(update_button("<hierarchy><node></hierarchy>", res), None);
        assert_eq!(update_button(OFFER, Resolution(100, 100)), None);
    }
    #[test]
    fn version_requires_all_installed_package_fields() {
        let text = "Package [com.supercell.clashofclans]\n versionCode=180647003 minSdk=24\n versionName=18.647.2\n lastUpdateTime=2026-10-24 02:39:47\n";
        assert_eq!(
            installed_version(text).as_deref(),
            Some("18.647.2 / 180647003 / 2026-10-24 02:39:47")
        );
        assert!(installed_version(&text.replace("lastUpdateTime", "firstInstallTime")).is_none());
    }
}
