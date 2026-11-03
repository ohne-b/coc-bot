use super::{Adb, Point, process::Transport};
use crate::{Error, Result, calibration::Resolution};
use roxmltree::{Document, ParsingOptions};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

const STORE: &str = "com.android.vending";

fn bounds(node: roxmltree::Node<'_, '_>, resolution: Resolution) -> Option<[u32; 4]> {
    let bounds = node
        .attribute("bounds")?
        .strip_prefix('[')?
        .strip_suffix(']')?;
    let (left, right) = bounds.split_once("][")?;
    let (x, y) = left.split_once(',')?;
    let (right, bottom) = right.split_once(',')?;
    let [x, y, right, bottom] = [x, y, right, bottom].map(str::parse::<u32>);
    let result = [x.ok()?, y.ok()?, right.ok()?, bottom.ok()?];
    (result[0] < result[2]
        && result[1] < result[3]
        && result[2] <= resolution.0
        && result[3] <= resolution.1)
        .then_some(result)
}

fn game_context(button: roxmltree::Node<'_, '_>, resolution: Resolution) -> bool {
    // The captured Store dialog and detail header associate the title with its own action group.
    let Some([left, top, right, _]) = bounds(button, resolution) else {
        return false;
    };
    button.document().descendants().any(|title| {
        if title.attribute("package") != Some(STORE) {
            return false;
        }
        if title.attribute("text") == Some("Clash of Clans") {
            let Some(header) = title.parent() else {
                return false;
            };
            let Some([x, _, end, bottom]) = bounds(header, resolution) else {
                return false;
            };
            header
                .descendants()
                .any(|node| node.attribute("text") == Some("Supercell"))
                && left >= x
                && right <= end
                && top >= bottom
                && top - bottom <= 250
        } else if title
            .attribute("content-desc")
            .and_then(|text| text.lines().next())
            == Some("App: Clash of Clans")
        {
            button
                .parent()
                .filter(|group| {
                    group.attribute("resource-id") == Some("com.android.vending:id/button_group")
                })
                .and_then(|group| group.parent())
                .is_some_and(|dialog| dialog.descendants().any(|node| node == title))
        } else {
            false
        }
    })
}

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
        if button.attribute("package") != Some(STORE)
            || button.attribute("enabled") != Some("true")
            || !game_context(button, resolution)
        {
            continue;
        }
        let [x, y, right, bottom] = bounds(button, resolution)?;
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
    const OFFER: &str = include_str!("../../tests/fixtures/playstore-update-offer.xml");
    const PAGE: &str = include_str!("../../tests/fixtures/playstore-game-page.xml");
    #[test]
    fn only_an_enabled_english_update_for_the_game_is_accepted() {
        let res = Resolution(1280, 720);
        let offer = OFFER.replace("Aplikasi: Clash of Clans", "App: Clash of Clans");
        assert_eq!(update_button(&offer, res), Some(Point(829, 497)));
        assert_eq!(update_button(OFFER, res), None);
        assert_eq!(update_button(PAGE, res), Some(Point(466, 466)));
        for (from, to) in [
            ("Update", "Install"),
            ("Update", "Perbarui"),
            ("Update", "Cancel"),
            ("Clash of Clans", "Other Game"),
            ("com.android.vending", "other.package"),
            ("enabled=\"true\"", "enabled=\"false\""),
        ] {
            assert_eq!(update_button(&offer.replace(from, to), res), None);
        }
        assert_eq!(update_button("<hierarchy><node></hierarchy>", res), None);
        assert_eq!(update_button(&offer, Resolution(100, 100)), None);
        let other_app = PAGE
            .replace("text=\"Clash of Clans\"", "text=\"Other Game\"")
            .replace("Crownstone Survival", "Clash of Clans");
        assert_eq!(update_button(&other_app, res), None);
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
