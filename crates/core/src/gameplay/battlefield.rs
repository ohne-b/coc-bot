use super::profile::Target;
use crate::{
    Error, Result,
    calibration::{Resolution, Screen},
    device::Point,
};
use opencv::core::Mat;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct Field {
    pub screen: Screen,
    pub name: String,
    pub origin: [i32; 2],
    pub u: [i32; 2],
    pub v: [i32; 2],
    pub anchors: Vec<Anchor>,
}
#[derive(Deserialize)]
pub struct Anchor {
    pub reference: Point,
    pub target: Target,
}

pub fn load(root: &Path) -> Result<Vec<Field>> {
    // shortcut: only captured sceneries are supported; add verified landmarks for other terrain.
    let path = root.join("battlefields.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut fields: Vec<Field> = serde_json::from_slice(&std::fs::read(path)?)?;
    if fields.len() > 32 {
        return Err(Error::Calibration("Too many battlefield profiles".into()));
    }
    for field in &mut fields {
        if field.anchors.len() < 3
            || field.anchors.len() > 8
            || !matches!(
                field.screen,
                Screen::HomeScout | Screen::BuilderScout | Screen::BuilderStage2
            )
            || field
                .origin
                .iter()
                .chain(field.u.iter())
                .chain(field.v.iter())
                .any(|n| n.unsigned_abs() > 2000)
            || (field.u[0] * field.v[1] - field.u[1] * field.v[0]).abs() < 10000
        {
            return Err(Error::Calibration("Invalid battlefield geometry".into()));
        }
        for anchor in &mut field.anchors {
            anchor.reference.validate(Resolution(1280, 720))?;
            anchor.target.load(root, Resolution(1280, 720))?;
        }
    }
    Ok(fields)
}
impl Field {
    fn supports(&self, screen: Screen) -> bool {
        self.screen == screen
            || (matches!(self.screen, Screen::BuilderScout | Screen::BuilderStage2)
                && matches!(screen, Screen::BuilderScout | Screen::BuilderStage2))
    }
    pub fn transform(
        &self,
        image: &Mat,
        hint: Option<(f64, [f64; 2])>,
    ) -> Result<Option<(f64, [f64; 2])>> {
        let mut pairs = Vec::new();
        for anchor in &self.anchors {
            let located = if let Some((scale, offset)) = hint {
                let predicted = [
                    f64::from(anchor.reference.0) * scale + offset[0],
                    f64::from(anchor.reference.1) * scale + offset[1],
                ];
                if predicted[0] < 0.
                    || predicted[1] < 0.
                    || predicted[0] >= 1280.
                    || predicted[1] >= 580.
                {
                    continue;
                }
                anchor.target.locate_near(
                    image,
                    Point(predicted[0].round() as u32, predicted[1].round() as u32),
                )?
            } else {
                anchor.target.locate_unique(image)?
            };
            if let Some(p) = located {
                pairs.push((
                    [f64::from(anchor.reference.0), f64::from(anchor.reference.1)],
                    [f64::from(p.0), f64::from(p.1)],
                ));
            }
        }
        // Three separated landmarks must agree on the camera's translation and scale.
        if pairs.len() < 3 {
            return Ok(None);
        }
        let n = pairs.len() as f64;
        let mean = |side: bool| {
            [0, 1].map(|axis| {
                pairs
                    .iter()
                    .map(|p| if side { p.1[axis] } else { p.0[axis] })
                    .sum::<f64>()
                    / n
            })
        };
        let a = mean(false);
        let b = mean(true);
        let variance = pairs
            .iter()
            .map(|(p, _)| (p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2))
            .sum::<f64>();
        if variance < 10000. {
            return Ok(None);
        }
        let scale = pairs
            .iter()
            .map(|(p, q)| (p[0] - a[0]) * (q[0] - b[0]) + (p[1] - a[1]) * (q[1] - b[1]))
            .sum::<f64>()
            / variance;
        if !(0.85..=1.1).contains(&scale) {
            return Ok(None);
        }
        let offset = [b[0] - a[0] * scale, b[1] - a[1] * scale];
        if pairs.iter().any(|(p, q)| {
            [0, 1]
                .iter()
                .any(|i| (p[*i] * scale + offset[*i] - q[*i]).abs() > 3.)
        }) {
            return Ok(None);
        }
        Ok(Some((scale, offset)))
    }
    pub fn project(&self, point: Point, (scale, offset): (f64, [f64; 2])) -> [i32; 2] {
        [0, 1].map(|i| {
            ((f64::from(self.origin[i])
                + f64::from(self.u[i]) * f64::from(point.0) / 1000.
                + f64::from(self.v[i]) * f64::from(point.1) / 1000.)
                * scale
                + offset[i])
                .round() as i32
        })
    }
}
pub fn identify(fields: &[Field], screen: Screen, image: &Mat) -> Result<usize> {
    let mut found = None;
    for (i, field) in fields
        .iter()
        .enumerate()
        .filter(|(_, f)| f.supports(screen))
    {
        if field.transform(image, None)?.is_some() {
            if found.is_some() {
                return Err(Error::Gameplay("Battlefield scenery is ambiguous".into()));
            }
            found = Some(i);
        }
    }
    found.ok_or_else(|| {
        Error::Gameplay(
            "Battlefield terrain is not calibrated for this view; no units deployed".into(),
        )
    })
}
pub fn locate(fields: &[Field], screen: Screen, image: &Mat, point: Point) -> Result<[i32; 2]> {
    if point.0 > 1000 || point.1 > 1000 {
        return Err(Error::InputGuard);
    }
    let mut result = None;
    for field in fields.iter().filter(|f| f.supports(screen)) {
        if let Some(transform) = field.transform(image, None)? {
            if result.is_some() {
                return Err(Error::Gameplay("Battlefield scenery is ambiguous".into()));
            }
            result = Some(field.project(point, transform));
        }
    }
    result.ok_or_else(|| {
        Error::Gameplay(
            "Battlefield terrain is not calibrated for this view; no units deployed".into(),
        )
    })
}
pub fn visible(p: [i32; 2]) -> Option<Point> {
    let [x, y] = p;
    ((80..=1200).contains(&x)
        && (90..=515).contains(&y)
        && !(x < 260 && y < 210)
        && !(x > 980 && y < 180)
        && !(y >= 470 && !(180..=1050).contains(&x)))
    .then_some(Point(x as u32, y as u32))
}
