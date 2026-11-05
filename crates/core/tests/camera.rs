use coc_bot_core::{
    calibration::Screen,
    device::{Gesture, Point},
    gameplay::{
        camera::{self, Camera, Normalization},
        profile::Profile,
    },
    vision::read_image,
};
use opencv::{
    core::{self, Mat, Rect, Scalar, Size},
    imgproc,
    prelude::*,
};
use std::path::Path;

fn frame(name: &str) -> Mat {
    read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("camera-{name}.png")),
    )
    .unwrap()
}
fn near(actual: [f64; 2], expected: [f64; 2]) {
    assert!(
        (actual[0] - expected[0]).abs() < 2. && (actual[1] - expected[1]).abs() < 2.,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn captured_camera_motion_handles_different_scenery_and_fixed_controls() {
    for (before, after, expected) in [
        ("home-start", "home-right", [60., 0.]),
        (
            "builder-diagonal-before",
            "builder-diagonal-after",
            [260., 170.],
        ),
        ("home-right", "home-down", [0., 108.]),
        ("home-down", "home-middle", [0., -104.]),
        ("home-middle", "home-up", [0., -170.]),
        ("home-other-start", "home-other-down", [55., 103.]),
        ("builder-start", "builder-right", [65., 0.]),
        ("builder-right", "builder-middle", [0., 156.]),
        ("builder-middle", "builder-down", [0., 63.]),
        ("builder-down", "builder-left", [-131., 0.]),
        ("builder-left", "builder-up", [0., -170.]),
        (
            "builder-clearing-edge-before",
            "builder-clearing-edge-after",
            [0., -170.],
        ),
        (
            "builder-cave-edge-before",
            "builder-cave-edge-after",
            [0., -170.],
        ),
    ] {
        near(
            camera::motion(&frame(before), &frame(after))
                .unwrap_or_else(|e| panic!("{before} -> {after}: {e}")),
            expected,
        );
    }
}

fn transformed(source: &Mat, scale: f64, x: f64, y: f64) -> Mat {
    let mut result = Mat::default();
    let matrix = Mat::from_slice_2d(&[[scale, 0., x], [0., scale, y]]).unwrap();
    imgproc::warp_affine(
        source,
        &mut result,
        &matrix,
        Size::new(1280, 720),
        imgproc::INTER_LINEAR,
        core::BORDER_CONSTANT,
        Scalar::all(0.),
    )
    .unwrap();
    result
}

#[test]
fn normalization_requires_actual_zoom_and_both_camera_limits() {
    let source = frame("home-start");
    let profile =
        Profile::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")).unwrap();
    let reference = profile
        .screen(Screen::HomeScout)
        .unwrap()
        .camera
        .as_ref()
        .unwrap();
    for ignored in [0, 1, 2, 3] {
        let mut routine = Normalization::default();
        let (mut scale, mut x, mut y) = (1.0_f64, 0.0_f64, 0.0_f64);
        let mut complete = false;
        for _ in 0..128 {
            let image = transformed(
                &source,
                scale,
                x + 640. * (1. - scale),
                y + 360. * (1. - scale),
            );
            match routine.next(&image) {
                Err(_) if ignored != 0 => {
                    complete = true;
                    break;
                }
                Err(e) => panic!("{e}"),
                Ok(camera::Adjustment::Ready) => {
                    assert_eq!(ignored, 0);
                    let geometry = routine.geometry(reference).unwrap();
                    near(geometry.pan_range, [487.5, 522.5]);
                    near(geometry.origin, reference.origin.map(|n| n * 1.25));
                    complete = true;
                    break;
                }
                Ok(camera::Adjustment::Wait) => {}
                Ok(camera::Adjustment::Move(gesture)) if ignored != 1 => match gesture {
                    Gesture::Pinch {
                        start_gap, end_gap, ..
                    } => {
                        scale = if end_gap == 420 {
                            1.25
                        } else if end_gap > start_gap {
                            1.3
                        } else {
                            1.0
                        };
                    }
                    Gesture::Pan { from, to } => {
                        if ignored == 2 && to.0 < from.0 {
                            continue;
                        }
                        x = (x + f64::from(to.0) - f64::from(from.0)).clamp(
                            -((1280. + 134.) * scale - 1280.) / 2.,
                            ((1280. + 134.) * scale - 1280.) / 2.,
                        );
                        if ignored == 3 && to.1 < from.1 {
                            continue;
                        }
                        y = (y + f64::from(to.1) - f64::from(from.1)).clamp(
                            -((720. + 274.) * scale - 720.) / 2.,
                            ((720. + 274.) * scale - 720.) / 2.,
                        );
                    }
                    _ => panic!("Unexpected gesture"),
                },
                Ok(camera::Adjustment::Move(_)) => {}
            }
        }
        assert!(complete);
    }
}

#[test]
fn diagonal_endpoints_are_certified_and_moving_probes_fall_back_to_axial_sweeps() {
    let source = frame("home-start");
    let profile =
        Profile::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")).unwrap();
    let reference = profile
        .screen(Screen::HomeScout)
        .unwrap()
        .camera
        .as_ref()
        .unwrap();
    for scenario in [
        "rectangle",
        "moving_probe",
        "wrong_return",
        "interrupted_return",
    ] {
        let mut routine = Normalization::default();
        let (mut scale, mut x, mut y) = (1.0_f64, 0.0_f64, 0.0_f64);
        let mut reversed = false;
        let mut outward_moves = 0;
        let mut negative_axial = 0;
        let mut finished = false;
        let mut moves = 0;
        let mut return_moves = 0;
        for observation in 1..=128 {
            let image = transformed(
                &source,
                scale,
                x + 640. * (1. - scale),
                y + 360. * (1. - scale),
            );
            match routine.next(&image) {
                Err(error) => {
                    assert!(
                        matches!(scenario, "wrong_return" | "interrupted_return"),
                        "{scenario}: {error}"
                    );
                    assert!(error.to_string().contains("certified endpoint"), "{error}");
                    finished = true;
                    break;
                }
                Ok(camera::Adjustment::Ready) => {
                    assert!(!matches!(scenario, "wrong_return" | "interrupted_return"));
                    let geometry = routine.geometry(reference).unwrap();
                    near(geometry.pan_range, [487.5, 522.5]);
                    if scenario == "moving_probe" {
                        assert!(outward_moves > 0 && negative_axial > 2);
                    } else {
                        assert_eq!(negative_axial, 2);
                        assert!(moves <= 22, "{moves} moves repeat endpoint input");
                        assert!(
                            observation <= 50,
                            "{observation} observations repeat settled endpoint checks"
                        );
                    }
                    finished = true;
                    break;
                }
                Ok(camera::Adjustment::Wait) => {}
                Ok(camera::Adjustment::Move(gesture)) => {
                    moves += 1;
                    match gesture {
                        Gesture::Pinch {
                            start_gap, end_gap, ..
                        } => {
                            scale = if end_gap == 420 {
                                1.25
                            } else if end_gap > start_gap {
                                1.3
                            } else {
                                1.0
                            };
                        }
                        Gesture::Pan { from, to } => {
                            let delta = [
                                f64::from(to.0) - f64::from(from.0),
                                f64::from(to.1) - f64::from(from.1),
                            ];
                            let diagonal = delta.iter().all(|n| *n != 0.);
                            if diagonal && delta[0] < 0. {
                                reversed = true;
                            }
                            if diagonal && delta[0] > 0. && reversed {
                                return_moves += 1;
                                if scenario == "interrupted_return" && return_moves == 2 {
                                    continue;
                                }
                            }
                            if !diagonal && delta.iter().any(|n| *n < 0.) {
                                negative_axial += 1;
                            }
                            let limits = [
                                ((1280. + 134.) * scale - 1280.) / 2.,
                                ((720. + 274.) * scale - 720.) / 2.,
                            ];
                            let right = if diagonal && delta[0] > 0. && scenario == "moving_probe" {
                                limits[0] - 30.
                            } else if diagonal
                                && delta[0] > 0.
                                && reversed
                                && scenario == "wrong_return"
                            {
                                limits[0] - 20.
                            } else {
                                limits[0]
                            };
                            let next_x = (x + delta[0]).clamp(-limits[0], right);
                            let next_y = (y + delta[1]).clamp(-limits[1], limits[1]);
                            if !diagonal && delta[0] > 0. && (next_x - x).abs() > 1. {
                                outward_moves += 1;
                            }
                            x = next_x;
                            y = next_y;
                        }
                        _ => panic!("Unexpected gesture"),
                    }
                }
            }
        }
        assert!(finished, "{scenario} did not finish");
    }
}
#[test]
fn missing_texture_changed_scale_and_other_battles_cannot_authorize_a_drop() {
    let source = frame("home-start");
    assert!(camera::motion(&source, &frame("home-other-start")).is_err());
    let blank = Mat::new_rows_cols_with_default(720, 1280, core::CV_8UC3, Scalar::all(0.)).unwrap();
    assert!(camera::motion(&blank, &blank).is_err());
    let mut hud = blank.clone();
    for r in [
        Rect::new(0, 0, 270, 210),
        Rect::new(980, 0, 300, 180),
        Rect::new(500, 0, 250, 80),
        Rect::new(0, 470, 610, 110),
        Rect::new(1050, 465, 230, 115),
        Rect::new(0, 580, 1280, 140),
    ] {
        Mat::roi(&source, r)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut hud, r).unwrap())
            .unwrap();
    }
    assert!(camera::motion(&hud, &hud).is_err());
    let mut zoomed = Mat::default();
    let transform = Mat::from_slice_2d(&[[1.1, 0., -64.], [0., 1.1, -36.]]).unwrap();
    imgproc::warp_affine(
        &source,
        &mut zoomed,
        &transform,
        Size::new(1280, 720),
        imgproc::INTER_LINEAR,
        core::BORDER_CONSTANT,
        Scalar::all(0.),
    )
    .unwrap();
    assert!(camera::motion(&source, &zoomed).is_err());
    let mut routine = Normalization::default();
    for _ in 0..5 {
        assert!(!matches!(
            routine.next(&blank).unwrap(),
            camera::Adjustment::Ready
        ));
    }
    assert!(routine.next(&blank).is_err());
}

#[test]
fn placements_follow_measured_pan_and_exclude_hud_buttons() {
    assert!(matches!(
        camera::pan_toward([573, 557]),
        Gesture::Pan {
            to: Point(707, 150),
            ..
        }
    ));
    let profile =
        Profile::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")).unwrap();
    let geometry = profile
        .screen(Screen::HomeScout)
        .unwrap()
        .camera
        .clone()
        .unwrap();
    let down = frame("home-down");
    let middle = frame("home-middle");
    let up = frame("home-up");
    let mut camera = Camera::new(geometry, &down).unwrap();
    assert_eq!(camera.position(&down, Point(0, 0)).unwrap(), [707, 75]);
    assert!(camera::visible([707, 75]).is_some());
    assert!(camera::visible([640, 573]).is_some());
    assert!(camera::visible([640, 582]).is_none());
    assert!(camera::visible(camera.position(&down, Point(1000, 1000)).unwrap()).is_none());
    camera.track(&middle).unwrap();
    camera.track(&up).unwrap();
    let corner = camera.position(&up, Point(1000, 1000)).unwrap();
    assert_eq!(
        camera.track_position(&up, Point(1000, 1000)).unwrap(),
        corner
    );
    assert!((corner[1] - 557).abs() <= 2, "{corner:?}");
    assert!(camera::visible(corner).is_some());
    assert!(camera.position(&up, Point(1001, 0)).is_err());
    for p in [
        [640, 50],
        [120, 100],
        [1100, 100],
        [280, 480],
        [495, 500],
        [1220, 410],
        [1190, 380],
        [100, 540],
        [450, 540],
        [1150, 520],
        [700, 600],
    ] {
        assert!(camera::visible(p).is_none(), "{p:?}");
    }
}

#[test]
fn deployment_fixtures_only_retain_documented_regions() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for file in ["deployment.json", "camera.json"] {
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixtures.join(file)).unwrap()).unwrap();
        for case in manifest["fixtures"].as_array().unwrap() {
            let file = case["file"].as_str().unwrap();
            let mut frame = read_image(&fixtures.join(file)).unwrap();
            let regions: Vec<coc_bot_core::calibration::Region> =
                serde_json::from_value(case["retained_regions"].clone()).unwrap();
            for coc_bot_core::calibration::Region(x, y, w, h) in regions {
                Mat::roi_mut(
                    &mut frame,
                    Rect::new(x as i32, y as i32, w as i32, h as i32),
                )
                .unwrap()
                .set_to(&Scalar::all(0.), &core::no_array())
                .unwrap();
            }
            assert!(
                frame.data_bytes().unwrap().iter().all(|v| *v == 0),
                "{file}"
            );
        }
    }
}
