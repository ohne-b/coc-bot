use crate::{
    Error, Result,
    device::{Gesture, Point},
};
use opencv::{
    calib3d,
    core::{self, DMatch, KeyPoint, Mat, Point2f, Rect, Scalar, Vector},
    features2d, imgproc,
    prelude::*,
};

#[derive(Clone, Copy, Debug)]
struct Motion {
    scale: f64,
    rotation: f64,
    offset: [f64; 2],
}
impl Motion {
    fn stopped(self) -> bool {
        (self.scale - 1.).abs() <= 0.003
            && self.rotation.abs() <= 0.003
            && self.offset.iter().all(|n| n.abs() <= 1.5)
    }
    fn apply(self, [x, y]: [f64; 2]) -> [f64; 2] {
        [
            self.scale * x - self.rotation * y + self.offset[0],
            self.rotation * x + self.scale * y + self.offset[1],
        ]
    }
    fn then(self, next: Self) -> Self {
        Self {
            scale: next.scale * self.scale - next.rotation * self.rotation,
            rotation: next.rotation * self.scale + next.scale * self.rotation,
            offset: next.apply(self.offset),
        }
    }
    fn at_execution_zoom(self) -> Result<Self> {
        if (self.scale - 1.).abs() > 0.005 || self.rotation.abs() > 0.003 {
            Err(Error::Gameplay(
                "Camera zoom changed during deployment".into(),
            ))
        } else {
            Ok(self)
        }
    }
}

fn register(before: &Mat, after: &Mat) -> Result<Motion> {
    let detector = features2d::ORB::create(
        8000,
        1.2,
        8,
        31,
        0,
        2,
        features2d::ORB_ScoreType::HARRIS_SCORE,
        31,
        5,
    )?;
    register_features(before, after, detector, core::NORM_HAMMING)
}

fn register_zoom(before: &Mat, after: &Mat) -> Result<Motion> {
    match register(before, after) {
        Err(Error::Gameplay(_)) => {
            // A large pinch can exceed ORB's scale correspondence; retain the same fit guards.
            let detector = features2d::SIFT::create(8000, 3, 0.04, 10., 1.6, false)?;
            register_features(before, after, detector, core::NORM_L2)
        }
        result => result,
    }
}

fn register_features(
    before: &Mat,
    after: &Mat,
    mut detector: impl features2d::Feature2DTrait,
    norm: i32,
) -> Result<Motion> {
    if before.size()? != core::Size::new(1280, 720)
        || after.size()? != before.size()?
        || before.typ() != core::CV_8UC3
        || after.typ() != core::CV_8UC3
    {
        return Err(Error::InputGuard);
    }
    let error = || {
        Error::Gameplay("Camera movement could not be verified; no deployment input sent".into())
    };
    let mut mask = Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC1, Scalar::all(0.))?;
    Mat::roi_mut(&mut mask, Rect::new(40, 90, 1200, 485))?
        .set_to(&Scalar::all(255.), &core::no_array())?;
    for r in [
        Rect::new(0, 0, 270, 210),
        Rect::new(980, 0, 300, 180),
        Rect::new(0, 470, 610, 110),
        Rect::new(1050, 465, 230, 115),
    ] {
        Mat::roi_mut(&mut mask, r)?.set_to(&Scalar::all(0.), &core::no_array())?;
    }
    let mut features = |image: &Mat| -> Result<(Vector<KeyPoint>, Mat)> {
        let mut gray = Mat::default();
        imgproc::cvt_color_def(image, &mut gray, imgproc::COLOR_BGR2GRAY)?;
        let mut detected = Vector::<KeyPoint>::new();
        detector.detect(&gray, &mut detected, &mask)?;
        let mut detected = detected.to_vec();
        detected.sort_by(|a, b| b.response().total_cmp(&a.response()));
        let mut cells = [0; 128];
        let mut points = Vector::new();
        // Small cells prevent one detailed building from exhausting a broad region of the budget.
        // At most 2816 features; acceptance still requires independent 200px by 100px evidence.
        for point in detected {
            let p = point.pt();
            let cell = (p.y as usize / 90) * 16 + (p.x as usize / 80);
            if cells[cell] < 22 {
                cells[cell] += 1;
                points.push(point);
            }
        }
        let mut descriptors = Mat::default();
        detector.compute(&gray, &mut points, &mut descriptors)?;
        Ok((points, descriptors))
    };
    let (a, da) = features(before)?;
    let (b, db) = features(after)?;
    if a.len() < 24 || b.len() < 24 {
        return Err(error());
    }
    let matcher = features2d::BFMatcher::new(norm, false)?;
    let mut forward: Vector<Vector<DMatch>> = Vector::new();
    let mut reverse: Vector<Vector<DMatch>> = Vector::new();
    matcher.knn_train_match_def(&da, &db, &mut forward, 2)?;
    matcher.knn_train_match_def(&db, &da, &mut reverse, 2)?;
    let mut from: Vector<Point2f> = Vector::new();
    let mut to: Vector<Point2f> = Vector::new();
    for pair in forward {
        if pair.len() != 2 {
            continue;
        }
        let m = pair.get(0)?;
        if m.distance >= pair.get(1)?.distance * 0.75 {
            continue;
        }
        let back = reverse.get(m.train_idx as usize)?;
        if back.len() != 2
            || back.get(0)?.train_idx != m.query_idx
            || back.get(0)?.distance >= back.get(1)?.distance * 0.75
        {
            continue;
        }
        from.push(a.get(m.query_idx as usize)?.pt());
        to.push(b.get(m.train_idx as usize)?.pt());
    }
    if from.len() < 16 {
        return Err(error());
    }
    let original = (from.clone(), to.clone());
    // ORB pyramid coordinates are quantized; refine matched corners before composing pan fits.
    for (image, points) in [(before, &mut from), (after, &mut to)] {
        let mut gray = Mat::default();
        imgproc::cvt_color_def(image, &mut gray, imgproc::COLOR_BGR2GRAY)?;
        imgproc::corner_sub_pix(
            &gray,
            points,
            core::Size::new(3, 3),
            core::Size::new(-1, -1),
            core::TermCriteria::new(
                core::TermCriteria_Type::COUNT as i32 | core::TermCriteria_Type::EPS as i32,
                30,
                0.01,
            )?,
        )?;
    }
    let motion = match fit_motion(&from, &to) {
        Err(Error::Gameplay(_)) => fit_motion(&original.0, &original.1),
        result => result,
    }?;
    if !(0.2..=4.).contains(&motion.scale) || motion.rotation.abs() > 0.1 {
        return Err(error());
    }
    Ok(motion)
}

fn fit_motion(from: &Vector<Point2f>, to: &Vector<Point2f>) -> Result<Motion> {
    let error = || {
        Error::Gameplay("Camera movement could not be verified; no deployment input sent".into())
    };
    let mut inliers = Mat::default();
    let fit = calib3d::estimate_affine_partial_2d(
        from,
        to,
        &mut inliers,
        calib3d::RANSAC,
        2.,
        2000,
        0.99,
        10,
    )?;
    if fit.empty() {
        return Err(error());
    }
    let accepted: Vec<_> = inliers
        .data_bytes()?
        .iter()
        .enumerate()
        .filter_map(|(i, v)| (*v != 0).then_some(i))
        .collect();
    if accepted.len() < 16 || accepted.len() * 5 < from.len() * 3 {
        return Err(error());
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for &i in &accepted {
        let p = from.get(i)?;
        for (axis, value) in [f64::from(p.x), f64::from(p.y)].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    if max[0] - min[0] < 200. || max[1] - min[1] < 100. {
        return Err(error());
    }
    let scale = *fit.at_2d::<f64>(0, 0)?;
    let rotation = *fit.at_2d::<f64>(1, 0)?;
    Ok(Motion {
        scale,
        rotation,
        offset: [*fit.at_2d::<f64>(0, 2)?, *fit.at_2d::<f64>(1, 2)?],
    })
}

/// Translation is only valid while the attack remains at the normalized zoom.
pub fn motion(before: &Mat, after: &Mat) -> Result<[f64; 2]> {
    Ok(register(before, after)?.at_execution_zoom()?.offset)
}
#[derive(Clone, serde::Deserialize)]
pub struct Geometry {
    pub pan_range: [f64; 2],
    pub origin: [f64; 2],
    pub u: [f64; 2],
    pub v: [f64; 2],
}
impl Geometry {
    pub fn validate(&self) -> Result<()> {
        if self
            .origin
            .iter()
            .chain(self.u.iter())
            .chain(self.v.iter())
            .chain(self.pan_range.iter())
            .any(|n| !n.is_finite() || n.abs() > 2000.)
            || self.pan_range.iter().any(|n| *n < 0.)
            || (self.u[0] * self.v[1] - self.u[1] * self.v[0]).abs() < 10000.
        {
            return Err(Error::Calibration(
                "Invalid normalized camera geometry".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Prime,
    Minimum,
    Zoom,
    SeekCorner,
    ProbeRight,
    ProbeDown,
    Reverse,
    ProbeLeft,
    ProbeUp,
    Return,
    Reattach,
    AxialRight,
    AxialLeft,
    AxialReturnRight,
    AxialDown,
    AxialUp,
    AxialReturnDown,
    Ready,
}
impl Phase {
    fn direction(self) -> Option<[i32; 2]> {
        Some(match self {
            Self::SeekCorner | Self::Return => [260, 170],
            Self::Reverse => [-260, -170],
            Self::ProbeRight | Self::AxialRight | Self::AxialReturnRight => [260, 0],
            Self::ProbeLeft | Self::AxialLeft => [-260, 0],
            Self::ProbeDown | Self::AxialDown | Self::AxialReturnDown => [0, 170],
            Self::ProbeUp | Self::AxialUp => [0, -170],
            _ => return None,
        })
    }
    fn probe(self) -> bool {
        matches!(
            self,
            Self::ProbeRight | Self::ProbeDown | Self::ProbeLeft | Self::ProbeUp
        )
    }
    fn measured(self) -> bool {
        matches!(
            self,
            Self::Reverse
                | Self::Return
                | Self::AxialLeft
                | Self::AxialReturnRight
                | Self::AxialUp
                | Self::AxialReturnDown
        )
    }
    fn next(self) -> Self {
        match self {
            Self::Prime => Self::Minimum,
            Self::Minimum => Self::Zoom,
            Self::Zoom => Self::SeekCorner,
            Self::SeekCorner => Self::ProbeRight,
            Self::ProbeRight => Self::ProbeDown,
            Self::ProbeDown => Self::Reverse,
            Self::Reverse => Self::ProbeLeft,
            Self::ProbeLeft => Self::ProbeUp,
            Self::ProbeUp => Self::Return,
            Self::Return => Self::Reattach,
            Self::Reattach => Self::Ready,
            Self::AxialRight => Self::AxialLeft,
            Self::AxialLeft => Self::AxialReturnRight,
            Self::AxialReturnRight => Self::AxialDown,
            Self::AxialDown => Self::AxialUp,
            Self::AxialUp => Self::AxialReturnDown,
            Self::AxialReturnDown | Self::Ready => Self::Ready,
        }
    }
}

#[derive(Clone, Default)]
pub struct Normalization {
    phase: Phase,
    attempts: u8,
    calls: u16,
    stable: u8,
    responded: [bool; 2],
    unreadable: u8,
    previous: Option<Mat>,
    endpoint: Option<Mat>,
    settling: Option<(Mat, u8, u8)>,
    travel: [f64; 2],
    reverse: [f64; 2],
    zoom: f64,
}

pub enum Adjustment {
    Move(Gesture),
    Wait,
    Ready,
}

impl Normalization {
    pub fn next(&mut self, image: &Mat) -> Result<Adjustment> {
        if self.phase == Phase::Ready {
            return Ok(Adjustment::Ready);
        }
        self.calls += 1;
        if self.calls > 128 {
            return Err(Error::Gameplay(
                "Camera normalization exceeded its observation limit".into(),
            ));
        }
        if self.phase == Phase::Reattach {
            return self.reattach(image);
        }
        let mut settled_pan = false;
        if let Some((previous, stable, captures)) = &mut self.settling {
            *captures += 1;
            if *captures > 8 {
                return Err(Error::Gameplay(
                    "Camera did not settle after panning".into(),
                ));
            }
            let movement = match register(previous, image) {
                Ok(movement) => movement.at_execution_zoom()?,
                Err(Error::Gameplay(_)) if self.unreadable < 3 => {
                    self.unreadable += 1;
                    return Ok(Adjustment::Wait);
                }
                Err(error) => return Err(error),
            };
            *stable = if movement.stopped() { *stable + 1 } else { 0 };
            *previous = image.clone();
            if *stable < 2 {
                return Ok(Adjustment::Wait);
            }
            self.settling = None;
            settled_pan = true;
        }
        if self.attempts > 0
            && let Some(previous) = &self.previous
        {
            let registration = if matches!(self.phase, Phase::Minimum | Phase::Zoom) {
                register_zoom(previous, image)
            } else {
                register(previous, image)
            };
            let movement = match registration {
                Ok(movement) => {
                    self.unreadable = 0;
                    movement
                }
                Err(Error::Gameplay(_)) if self.unreadable < 3 => {
                    self.unreadable += 1;
                    return Ok(Adjustment::Wait);
                }
                Err(error) => return Err(error),
            };
            self.stable = if movement.stopped() {
                self.stable + 1
            } else {
                0
            };
            // The diagonal path still certifies its endpoints with cardinal probes,
            // opposite range measurements and reattachment, without repeating a blocked pan.
            let settled_endpoint = settled_pan
                && movement.stopped()
                && (self.phase.probe()
                    || matches!(
                        self.phase,
                        Phase::SeekCorner | Phase::Reverse | Phase::Return
                    ));
            if movement.rotation.abs() > 0.003 {
                return Err(Error::Gameplay(
                    "Camera rotated unexpectedly while positioning".into(),
                ));
            }
            if self.phase == Phase::Zoom {
                self.zoom *= movement.scale;
                if self.zoom > 1.6 || (self.stable >= 2 && self.zoom < 1.22) {
                    return Err(Error::Gameplay(
                        "Camera zoom did not reach a usable view".into(),
                    ));
                }
                if self.zoom >= 1.22 {
                    self.advance();
                }
            } else {
                if self.phase == Phase::Minimum {
                    self.responded[0] |= movement.scale < 0.98;
                }
                if let Some(direction) = self.phase.direction() {
                    movement.at_execution_zoom()?;
                    for (axis, step) in direction.into_iter().enumerate() {
                        let distance = movement.offset[axis] * f64::from(step.signum());
                        // Read actual settled movement, not finger distance: captured Home pans
                        // moved up to365x266 for a260x170 gesture. Keep bounded overlap.
                        let bound = if axis == 0 { 400. } else { 300. };
                        let unexpected = if self.phase.probe() {
                            movement.offset[axis].abs() > bound
                        } else {
                            (step == 0 && movement.offset[axis].abs() > 3.)
                                || distance < -3.
                                || distance > bound
                        };
                        if unexpected {
                            return Err(Error::Gameplay(
                                "Camera moved unexpectedly while positioning".into(),
                            ));
                        }
                        if self.phase.measured() && step != 0 {
                            self.responded[axis] |= distance > 10.;
                            if step < 0 {
                                self.reverse[axis] += distance;
                            } else {
                                self.travel[axis] += distance;
                            }
                        }
                    }
                }
                if self.phase.probe() && !movement.stopped() {
                    self.travel = [0.; 2];
                    self.reverse = [0.; 2];
                    self.endpoint = None;
                    self.enter(Phase::AxialRight);
                } else if self.phase.probe() && self.stable < 2 && !settled_endpoint {
                    // Reobserve the endpoint after its outward probe without another gesture.
                    return Ok(Adjustment::Wait);
                }
                if self.stable >= 2 || settled_endpoint {
                    let missing_response = self.phase == Phase::Minimum && !self.responded[0]
                        || self.phase.measured()
                            && self.phase.direction().is_some_and(|direction| {
                                (0..2).any(|axis| direction[axis] != 0 && !self.responded[axis])
                            });
                    if missing_response {
                        return Err(Error::Gameplay(
                            "Camera did not respond to zoom or pan input".into(),
                        ));
                    }
                    if self.phase == Phase::ProbeDown {
                        self.endpoint = Some(image.clone());
                    }
                    self.advance();
                }
            }
        }
        if self.phase == Phase::Ready {
            return Ok(Adjustment::Ready);
        }
        if self.phase == Phase::Reattach {
            return self.reattach(image);
        }
        if self.attempts >= 8 {
            return Err(Error::Gameplay(
                "Camera did not reach a stable zoom or pan limit".into(),
            ));
        }
        self.previous = Some(image.clone());
        if self.phase == Phase::Prime {
            self.phase = Phase::Minimum;
            return Ok(Adjustment::Move(Gesture::Pinch {
                center: Point(640, 360),
                start_gap: 300,
                end_gap: 450,
            }));
        }
        self.attempts += 1;
        Ok(Adjustment::Move(match self.phase {
            Phase::Minimum => Gesture::Pinch {
                center: Point(640, 360),
                start_gap: 400,
                end_gap: 200,
            },
            Phase::Zoom => Gesture::Pinch {
                center: Point(640, 360),
                start_gap: 300,
                end_gap: 420,
            },
            phase => {
                let [x, y] = phase.direction().ok_or(Error::InputGuard)?;
                self.settling = Some((image.clone(), 0, 0));
                Gesture::Pan {
                    from: Point(640, 320),
                    to: Point((640 + x) as u32, (320 + y) as u32),
                }
            }
        }))
    }
    fn reattach(&mut self, image: &Mat) -> Result<Adjustment> {
        match register(self.endpoint.as_ref().ok_or(Error::InputGuard)?, image) {
            Ok(movement) if movement.stopped() => {
                self.enter(Phase::Ready);
                Ok(Adjustment::Ready)
            }
            Err(Error::Gameplay(_)) if self.unreadable < 3 => {
                self.unreadable += 1;
                Ok(Adjustment::Wait)
            }
            Err(error) => Err(error),
            Ok(_) => Err(Error::Gameplay(
                "Camera did not return to the certified endpoint".into(),
            )),
        }
    }
    fn advance(&mut self) {
        self.enter(self.phase.next());
    }
    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.settling = None;
        self.attempts = 0;
        self.stable = 0;
        self.responded = [false; 2];
        self.unreadable = 0;
        if self.phase == Phase::Zoom {
            self.zoom = 1.;
        }
    }
    pub fn geometry(&self, reference: &Geometry) -> Result<Geometry> {
        if self.phase != Phase::Ready
            || !(10.0..=1000.).contains(&self.travel[0])
            || !(50.0..=1200.).contains(&self.travel[1])
            || (0..2).any(|axis| (self.travel[axis] - self.reverse[axis]).abs() > 3.)
        {
            return Err(Error::Gameplay(
                "The full camera range could not be measured".into(),
            ));
        }
        reference.validate()?;
        let scale = (1280. + self.travel[0]) / (1280. + reference.pan_range[0]);
        let vertical_scale = (720. + self.travel[1]) / (720. + reference.pan_range[1]);
        if (scale - vertical_scale).abs() > 0.01 {
            return Err(Error::Gameplay(
                "Camera bounds do not match this village".into(),
            ));
        }
        let u = reference.u.map(|n| n * scale);
        let v = reference.v.map(|n| n * scale);
        let origin = [0, 1].map(|axis| {
            let middle = if axis == 0 { 640. } else { 360. };
            let center = reference.origin[axis] + (reference.u[axis] + reference.v[axis]) / 2.;
            middle
                + (center - reference.pan_range[axis] / 2. - middle) * scale
                + self.travel[axis] / 2.
                - (u[axis] + v[axis]) / 2.
        });
        Ok(Geometry {
            origin,
            u,
            v,
            pan_range: self.travel,
        })
    }
}

pub struct Camera {
    geometry: Geometry,
    origin: Mat,
    reference: Mat,
    transform: Motion,
}
impl Camera {
    pub fn new(geometry: Geometry, image: &Mat) -> Result<Self> {
        geometry.validate()?;
        motion(image, image)?;
        Ok(Self {
            geometry,
            origin: image.clone(),
            reference: image.clone(),
            transform: Motion {
                scale: 1.,
                rotation: 0.,
                offset: [0., 0.],
            },
        })
    }
    pub fn position(&self, image: &Mat, point: Point) -> Result<[i32; 2]> {
        self.project(self.transform(image)?, point)
    }
    pub fn track_position(&mut self, image: &Mat, point: Point) -> Result<[i32; 2]> {
        self.track(image)?;
        self.project(self.transform, point)
    }
    fn project(&self, transform: Motion, point: Point) -> Result<[i32; 2]> {
        if point.0 > 1000 || point.1 > 1000 {
            return Err(Error::InputGuard);
        }
        Ok(transform
            .apply([0, 1].map(|i| {
                self.geometry.origin[i]
                    + self.geometry.u[i] * f64::from(point.0) / 1000.
                    + self.geometry.v[i] * f64::from(point.1) / 1000.
            }))
            .map(|n| n.round() as i32))
    }
    pub fn track(&mut self, image: &Mat) -> Result<()> {
        self.transform = self.transform(image)?;
        self.reference = image.clone();
        Ok(())
    }
    fn transform(&self, image: &Mat) -> Result<Motion> {
        let tracked = match register(&self.reference, image) {
            Ok(movement) => self
                .transform
                .then(movement.at_execution_zoom()?)
                .at_execution_zoom()?,
            Err(Error::Gameplay(_)) => return register(&self.origin, image)?.at_execution_zoom(),
            Err(error) => return Err(error),
        };
        // A distant original view can fit animated scenery; reattach only when both poses agree.
        if let Ok(anchor) = register(&self.origin, image).and_then(Motion::at_execution_zoom)
            && [[0., 0.], [1., 0.], [0., 1.], [1., 1.]]
                .into_iter()
                .all(|[u, v]| {
                    let point = [0, 1].map(|i| {
                        self.geometry.origin[i] + self.geometry.u[i] * u + self.geometry.v[i] * v
                    });
                    let a = anchor.apply(point);
                    let b = tracked.apply(point);
                    (0..2).all(|i| (a[i] - b[i]).abs() <= 3.)
                })
        {
            return Ok(anchor);
        }
        Ok(tracked)
    }
}

pub fn pan_toward([x, y]: [i32; 2]) -> Gesture {
    // Move both axes: a vertical limit can leave a corner beneath the horizontal HUD.
    Gesture::Pan {
        from: Point(640, 320),
        to: Point(
            (640 + 640_i32.saturating_sub(x).clamp(-260, 260)) as u32,
            (320 + 320_i32.saturating_sub(y).clamp(-170, 170)) as u32,
        ),
    }
}

pub fn visible([x, y]: [i32; 2]) -> Option<Point> {
    ((80..=1200).contains(&x)
        && (65..=578).contains(&y)
        && !(x < 260 && y < 210)
        && !(x > 980 && y < 180)
        && !(y < 80 && (530..=700).contains(&x))
        && !(y >= 350 && x > 1160)
        && !(y >= 470 && x > 1050)
        && !(y >= 460 && x < 610))
        .then_some(Point(x as u32, y as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large_zoom_view(index: u8) -> Mat {
        crate::vision::read_image(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/camera-home-large-zoom-{index:03}.png"
            )),
        )
        .unwrap()
    }

    #[test]
    fn captured_large_pinch_uses_scale_features_without_weakening_runtime_pan_checks() {
        let before = large_zoom_view(3);
        for index in 4..=7 {
            let after = large_zoom_view(index);
            assert!(register(&before, &after).is_err());
            let movement = register_zoom(&before, &after).unwrap();
            assert!((movement.scale - 0.5204).abs() < 0.002);
            assert!(movement.rotation.abs() < 0.001);
            assert!((movement.offset[0] - 307.).abs() < 2.);
            assert!((movement.offset[1] - 172.).abs() < 2.);
            assert!(motion(&before, &after).is_err());
            let mut routine = Normalization {
                phase: Phase::Minimum,
                attempts: 1,
                previous: Some(before.clone()),
                ..Default::default()
            };
            assert!(matches!(
                routine.next(&after).unwrap(),
                Adjustment::Move(Gesture::Pinch { end_gap: 200, .. })
            ));
            assert!(routine.responded[0]);
            assert!(routine.phase == Phase::Minimum);
            assert_eq!(routine.attempts, 2);
        }
    }

    #[test]
    fn zoom_scale_fallback_still_rejects_unrelated_sparse_and_rotated_views() {
        let before = large_zoom_view(3);
        let unrelated = crate::vision::read_image(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/camera-home-other-start.png"),
        )
        .unwrap();
        let blank =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        for missing in [&unrelated, &blank] {
            assert!(register_zoom(&before, missing).is_err());
            let mut routine = Normalization {
                phase: Phase::Minimum,
                attempts: 1,
                previous: Some(before.clone()),
                ..Default::default()
            };
            for _ in 0..3 {
                assert!(matches!(routine.next(missing).unwrap(), Adjustment::Wait));
            }
            assert!(routine.next(missing).is_err());
            assert_eq!(routine.attempts, 1);
            assert!(!routine.responded[0]);
        }
        let mut rotated = Mat::default();
        imgproc::warp_affine(
            &large_zoom_view(4),
            &mut rotated,
            &imgproc::get_rotation_matrix_2d(Point2f::new(640., 360.), 1., 1.).unwrap(),
            core::Size::new(1280, 720),
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.),
        )
        .unwrap();
        let mut routine = Normalization {
            phase: Phase::Minimum,
            attempts: 1,
            previous: Some(before),
            ..Default::default()
        };
        assert!(
            matches!(routine.next(&rotated), Err(Error::Gameplay(message)) if message.contains("rotated unexpectedly"))
        );
    }

    #[test]
    fn distant_anchor_cannot_replace_verified_recent_pan_motion() {
        let load = |name: &str| {
            crate::vision::read_image(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/camera-builder-cave-pan-{name}.png")),
            )
            .unwrap()
        };
        let origin = load("058-engine");
        let guarded = load("063-rejected");
        assert!(
            register(&origin, &guarded)
                .unwrap()
                .at_execution_zoom()
                .is_err()
        );
        let mut camera = Camera::new(
            Geometry {
                origin: [640., 360.],
                u: [400., 200.],
                v: [-400., 200.],
                pan_range: [1000., 1000.],
            },
            &origin,
        )
        .unwrap();
        for name in ["061-engine", "062-engine", "063-engine"] {
            camera.track(&load(name)).unwrap();
        }
        let pose = camera.transform(&guarded).unwrap();
        assert!(pose.rotation.abs() < 0.001);
        assert!((pose.scale - 1.).abs() < 0.003);
        assert!(pose.offset[0].abs() < 2. && (pose.offset[1] + 341.).abs() < 2.);
        // The unchanged recent pose must not hide an actual change in zoom or rotation.
        for matrix in [
            Mat::from_slice_2d(&[[1.02, 0., -12.8], [0., 1.02, -7.2]]).unwrap(),
            imgproc::get_rotation_matrix_2d(core::Point2f::new(640., 360.), 1., 1.).unwrap(),
        ] {
            let mut changed = Mat::default();
            imgproc::warp_affine(
                &guarded,
                &mut changed,
                &matrix,
                core::Size::new(1280, 720),
                imgproc::INTER_LINEAR,
                core::BORDER_CONSTANT,
                Scalar::all(0.),
            )
            .unwrap();
            assert!(camera.transform(&changed).is_err());
        }
    }
    fn captured_view() -> Mat {
        crate::vision::read_image(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/camera-home-start.png"),
        )
        .unwrap()
    }

    fn blocked_pan(phase: Phase, image: &Mat) -> Normalization {
        Normalization {
            phase,
            attempts: 1,
            responded: [true; 2],
            previous: Some(image.clone()),
            settling: Some((image.clone(), 0, 0)),
            endpoint: Some(image.clone()),
            ..Default::default()
        }
    }

    #[test]
    fn settled_blocked_pans_keep_each_independent_endpoint_check() {
        let image = captured_view();
        for (phase, next) in [
            (Phase::SeekCorner, Phase::ProbeRight),
            (Phase::ProbeRight, Phase::ProbeDown),
            (Phase::ProbeDown, Phase::Reverse),
            (Phase::Reverse, Phase::ProbeLeft),
            (Phase::ProbeLeft, Phase::ProbeUp),
            (Phase::ProbeUp, Phase::Return),
        ] {
            let mut routine = blocked_pan(phase, &image);
            assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
            assert!(routine.phase == phase);
            assert!(matches!(
                routine.next(&image).unwrap(),
                Adjustment::Move(Gesture::Pan { .. })
            ));
            assert!(routine.phase == next);
            assert_eq!(routine.attempts, 1, "Only the next phase sends input");
        }
        let mut routine = blocked_pan(Phase::Return, &image);
        assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
        assert!(matches!(routine.next(&image).unwrap(), Adjustment::Ready));
        assert!(routine.phase == Phase::Ready);
    }

    #[test]
    fn minimum_zoom_and_axial_sweeps_still_require_two_blocked_gestures() {
        let image = captured_view();
        for phase in [
            Phase::AxialRight,
            Phase::AxialLeft,
            Phase::AxialReturnRight,
            Phase::AxialDown,
            Phase::AxialUp,
            Phase::AxialReturnDown,
        ] {
            let mut routine = blocked_pan(phase, &image);
            assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
            assert!(matches!(
                routine.next(&image).unwrap(),
                Adjustment::Move(Gesture::Pan { .. })
            ));
            assert!(routine.phase == phase);
            assert_eq!(routine.attempts, 2);
            assert_eq!(routine.stable, 1);
        }
        let mut routine = blocked_pan(Phase::Minimum, &image);
        routine.settling = None;
        assert!(matches!(
            routine.next(&image).unwrap(),
            Adjustment::Move(Gesture::Pinch { end_gap: 200, .. })
        ));
        assert!(routine.phase == Phase::Minimum);
        assert_eq!(routine.attempts, 2);
    }

    #[test]
    fn settled_endpoint_cannot_replace_response_or_reattachment_evidence() {
        let image = captured_view();
        for phase in [Phase::Reverse, Phase::Return] {
            for responded in [[false, false], [true, false], [false, true]] {
                let mut routine = blocked_pan(phase, &image);
                routine.responded = responded;
                assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
                assert!(
                    matches!(routine.next(&image), Err(Error::Gameplay(message)) if message.contains("did not respond"))
                );
            }
        }
        let mut routine = blocked_pan(Phase::Return, &image);
        routine.endpoint = Some(
            crate::vision::read_image(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/camera-home-right.png"),
            )
            .unwrap(),
        );
        assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
        assert!(
            matches!(routine.next(&image), Err(Error::Gameplay(message)) if message.contains("certified endpoint"))
        );
    }

    #[test]
    fn delayed_pan_motion_cannot_be_mistaken_for_a_blocked_gesture() {
        let image = captured_view();
        let moved = crate::vision::read_image(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/camera-home-right.png"),
        )
        .unwrap();
        for phase in [Phase::Return, Phase::ProbeRight] {
            let mut routine = blocked_pan(phase, &image);
            assert!(matches!(routine.next(&image).unwrap(), Adjustment::Wait));
            for _ in 0..2 {
                assert!(matches!(routine.next(&moved).unwrap(), Adjustment::Wait));
                assert_eq!(routine.travel, [0.; 2]);
            }
            assert!(matches!(
                routine.next(&moved).unwrap(),
                Adjustment::Move(Gesture::Pan { .. })
            ));
            if phase == Phase::Return {
                assert!(routine.phase == Phase::Return);
                assert!((routine.travel[0] - 60.).abs() < 2.);
            } else {
                assert!(routine.phase == Phase::AxialRight);
                assert_eq!(routine.travel, [0.; 2]);
            }
        }
    }

    #[test]
    fn captured_pan_settles_without_more_input_and_counts_actual_travel_once() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let load = |n| {
            crate::vision::read_image(&fixtures.join(format!("camera-home-settle-{n}.png")))
                .unwrap()
        };
        let before = load(27);
        let mut routine = Normalization {
            phase: Phase::Return,
            attempts: 1,
            previous: Some(before.clone()),
            settling: Some((before, 0, 0)),
            ..Default::default()
        };
        // Three captured moving observations, then two unchanged observations.
        for n in [28, 29, 30, 30] {
            assert!(matches!(routine.next(&load(n)).unwrap(), Adjustment::Wait));
            assert_eq!(routine.travel, [0.; 2]);
            assert_eq!(routine.attempts, 1);
        }
        assert!(matches!(
            routine.next(&load(30)).unwrap(),
            Adjustment::Move(Gesture::Pan { .. })
        ));
        assert!(
            (routine.travel[0] - 365.).abs() < 2. && (routine.travel[1] - 266.).abs() < 2.,
            "{:?}",
            routine.travel
        );
        assert!(matches!(routine.phase, Phase::Return));
        assert_eq!(
            routine.stable, 0,
            "A moving gesture settling is not an endpoint"
        );
        assert_eq!(routine.attempts, 2);
        let travel = routine.travel;
        assert!(matches!(routine.next(&load(30)).unwrap(), Adjustment::Wait));
        assert_eq!(routine.travel, travel);
    }

    #[test]
    fn pan_settling_is_bounded_and_rejects_zoom_reversal_and_excess_travel() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let before = crate::vision::read_image(&fixtures.join("camera-home-start.png")).unwrap();
        let transform = |scale, x, y| {
            let mut image = Mat::default();
            let matrix = Mat::from_slice_2d(&[[scale, 0., x], [0., scale, y]]).unwrap();
            imgproc::warp_affine(
                &before,
                &mut image,
                &matrix,
                core::Size::new(1280, 720),
                imgproc::INTER_LINEAR,
                core::BORDER_CONSTANT,
                Scalar::all(0.),
            )
            .unwrap();
            image
        };
        for scenario in ["moving", "zoom", "reverse", "excess", "unknown"] {
            let mut routine = Normalization {
                phase: Phase::Return,
                attempts: 1,
                previous: Some(before.clone()),
                settling: Some((before.clone(), 0, 0)),
                ..Default::default()
            };
            let mut stopped = false;
            for n in 1..=9 {
                let image = match scenario {
                    "moving" => transform(1., f64::from(n) * 5., 0.),
                    "zoom" => transform(1.02, 0., 0.),
                    "reverse" => transform(1., -20., 20.),
                    "excess" => transform(1., 410., 0.),
                    _ => Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.))
                        .unwrap(),
                };
                match routine.next(&image) {
                    Ok(Adjustment::Wait) => {}
                    Err(_) => {
                        stopped = true;
                        break;
                    }
                    Ok(_) => panic!("{scenario} must not authorize another gesture"),
                }
            }
            assert!(stopped, "{scenario}");
            assert_eq!(routine.travel, [0.; 2]);
        }
    }

    #[test]
    fn captured_diagonal_return_requires_spatially_distributed_evidence() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let before =
            crate::vision::read_image(&fixtures.join("camera-builder-diagonal-before.png"))
                .unwrap();
        let after =
            crate::vision::read_image(&fixtures.join("camera-builder-diagonal-after.png")).unwrap();
        let mut routine = Normalization {
            phase: Phase::Return,
            attempts: 1,
            previous: Some(before.clone()),
            ..Default::default()
        };
        assert!(matches!(
            routine.next(&after).unwrap(),
            Adjustment::Move(Gesture::Pan { .. })
        ));
        assert!((routine.travel[0] - 260.).abs() < 2. && (routine.travel[1] - 170.).abs() < 2.);
        assert_eq!(routine.unreadable, 0);
        // The formerly dominant building alone still cannot authorize movement.
        let mut narrow_before =
            Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
        let mut narrow_after = narrow_before.try_clone().unwrap();
        for (source, target, rect) in [
            (&before, &mut narrow_before, Rect::new(295, 91, 168, 147)),
            (&after, &mut narrow_after, Rect::new(555, 261, 168, 147)),
        ] {
            Mat::roi(source, rect)
                .unwrap()
                .copy_to(&mut Mat::roi_mut(target, rect).unwrap())
                .unwrap();
        }
        assert!(register(&narrow_before, &narrow_after).is_err());
    }

    #[test]
    fn axial_fallback_cannot_reset_the_overall_observation_bound() {
        let mut routine = Normalization {
            phase: Phase::ProbeRight,
            calls: 128,
            ..Default::default()
        };
        routine.enter(Phase::AxialRight);
        assert!(
            matches!(routine.next(&Mat::default()), Err(Error::Gameplay(message)) if message.contains("observation limit"))
        );
    }

    #[test]
    fn captured_active_builder_pans_do_not_accumulate_false_rotation() {
        let load = |n| {
            crate::vision::read_image(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/camera-builder-active-pan-{n}.png")),
            )
            .unwrap()
        };
        let origin = load(236);
        let mut camera = Camera::new(
            Geometry {
                origin: [640., 360.],
                u: [400., 200.],
                v: [-400., 200.],
                pan_range: [1000., 1000.],
            },
            &origin,
        )
        .unwrap();
        for n in [247, 248, 249, 254, 259, 260] {
            camera.track(&load(n)).unwrap();
        }
        assert!(camera.transform.rotation.abs() < 0.001);
        assert!((camera.transform.scale - 1.).abs() < 0.003);
        // Independently matched static patches agree on this pure translation despite moving troops.
        let fit = register(&load(248), &load(249)).unwrap();
        for point in [
            [380., 230.],
            [860., 230.],
            [1040., 330.],
            [620., 430.],
            [1100., 430.],
        ] {
            let actual = fit.apply(point);
            assert!((actual[0] - (point[0] - 183.)).abs() < 2.);
            assert!((actual[1] - (point[1] - 106.)).abs() < 2.);
        }
        let final_image = load(260);
        let mut changed = Mat::default();
        let zoom = Mat::from_slice_2d(&[[1.02, 0., -12.8], [0., 1.02, -7.2]]).unwrap();
        imgproc::warp_affine(
            &final_image,
            &mut changed,
            &zoom,
            core::Size::new(1280, 720),
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.),
        )
        .unwrap();
        assert!(
            camera.track(&changed).is_err(),
            "Real zoom must still stop deployment"
        );
        let rotation =
            imgproc::get_rotation_matrix_2d(core::Point2f::new(640., 360.), 1., 1.).unwrap();
        imgproc::warp_affine(
            &final_image,
            &mut changed,
            &rotation,
            core::Size::new(1280, 720),
            imgproc::INTER_LINEAR,
            core::BORDER_CONSTANT,
            Scalar::all(0.),
        )
        .unwrap();
        assert!(
            camera.track(&changed).is_err(),
            "Real rotation must still stop deployment"
        );
    }

    #[test]
    fn rolling_registration_preserves_small_motion_and_rejects_accumulated_zoom() {
        let mut transform = Motion {
            scale: 1.,
            rotation: 0.,
            offset: [0., 0.],
        };
        for _ in 0..50 {
            transform = transform.then(Motion {
                scale: 1.,
                rotation: 0.,
                offset: [0.2, -0.1],
            });
        }
        assert!((transform.offset[0] - 10.).abs() < 0.001);
        assert!((transform.offset[1] + 5.).abs() < 0.001);
        let small_zoom = Motion {
            scale: 1.004,
            rotation: 0.,
            offset: [-2.56, -1.44],
        };
        assert!(small_zoom.at_execution_zoom().is_ok());
        assert!(small_zoom.then(small_zoom).at_execution_zoom().is_err());
    }
    #[test]
    fn measured_geometry_matches_independently_reviewed_grass_corners() {
        use crate::{calibration::Screen, gameplay::profile::Profile, vision::read_image};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixtures = root.join("tests/fixtures");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join("camera.json")).unwrap()).unwrap();
        let case = &manifest["home_raised"];
        let range: [f64; 2] = serde_json::from_value(case["pan_range"].clone()).unwrap();
        let routine = Normalization {
            phase: Phase::Ready,
            travel: range,
            reverse: range,
            ..Default::default()
        };
        let profile = Profile::load(&root.join("../../assets")).unwrap();
        let reference = profile
            .screen(Screen::HomeScout)
            .unwrap()
            .camera
            .as_ref()
            .unwrap();
        let geometry = routine.geometry(reference).unwrap();
        let origin = read_image(&fixtures.join(case["origin_frame"].as_str().unwrap())).unwrap();
        let mut camera = Camera::new(geometry, &origin).unwrap();
        for file in case["tracking_frames"].as_array().unwrap() {
            let image = read_image(&fixtures.join(file.as_str().unwrap())).unwrap();
            camera
                .track(&image)
                .unwrap_or_else(|e| panic!("{file}: {e}"));
            let Some(placement) = case["placements"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["file"] == *file)
            else {
                continue;
            };
            let point: Point = serde_json::from_value(placement["point"].clone()).unwrap();
            let region: [i32; 4] = serde_json::from_value(placement["region"].clone()).unwrap();
            let [x, y] = camera.position(&image, point).unwrap();
            assert!(
                x >= region[0]
                    && x < region[0] + region[2]
                    && y >= region[1]
                    && y < region[1] + region[3],
                "{point:?} at [{x},{y}]"
            );
        }
        let mut ignored = routine.clone();
        ignored.reverse[0] -= 50.;
        assert!(ignored.geometry(reference).is_err());
        let mut inconsistent = routine;
        inconsistent.travel[1] += 100.;
        inconsistent.reverse[1] += 100.;
        assert!(inconsistent.geometry(reference).is_err());
    }
}
