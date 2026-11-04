use coc_bot_core::{calibration::Screen, device::Point, gameplay::battlefield, vision::read_image};
use opencv::{
    core::{self, Mat, Rect, Scalar, Size},
    imgproc,
    prelude::*,
};
use std::path::Path;

#[test]
fn fixed_scout_controls_are_not_treated_as_moving_terrain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let fields = battlefield::load(&root).unwrap();
    let field = fields.iter().find(|f| f.name == "haunted").unwrap();
    let source = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/battlefield-haunted.png"),
    )
    .unwrap();
    let control = Rect::new(270, 538, 35, 35);
    let mut terrain = source.clone();
    Mat::roi_mut(&mut terrain, control)
        .unwrap()
        .set_to(&Scalar::all(0.), &core::no_array())
        .unwrap();
    let mut moved = Mat::default();
    let translation = Mat::from_slice_2d(&[[1.0, 0.0, 0.0], [0.0, 1.0, -80.0]]).unwrap();
    imgproc::warp_affine(
        &terrain,
        &mut moved,
        &translation,
        Size::new(1280, 720),
        imgproc::INTER_NEAREST,
        core::BORDER_CONSTANT,
        Scalar::all(0.),
    )
    .unwrap();
    // Camera movement leaves the Boost Army control fixed on screen.
    Mat::roi(&source, control)
        .unwrap()
        .copy_to(&mut Mat::roi_mut(&mut moved, control).unwrap())
        .unwrap();
    let (scale, offset) = field.transform(&moved, None).unwrap().unwrap();
    assert!((scale - 1.).abs() < 0.01);
    assert!(offset[0].abs() < 2. && (offset[1] + 80.).abs() < 2.);
    let live = read_image(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/battlefield-haunted-battle.png"),
    )
    .unwrap();
    let transform = field.transform(&live, None).unwrap().unwrap();
    assert!((0.96..=0.98).contains(&transform.0));
    assert!(battlefield::visible(field.project(Point(0, 0), transform)).is_none());
}

#[test]
fn captured_terrain_tracks_camera_translation_and_rejects_unknown_scenery() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let fields = battlefield::load(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let before = read_image(&fixtures.join("battlefield-classic.png")).unwrap();
    let after = read_image(&fixtures.join("battlefield-classic-panned.png")).unwrap();
    let a = battlefield::locate(&fields, Screen::HomeScout, &before, Point(0, 500)).unwrap();
    let b = battlefield::locate(&fields, Screen::HomeScout, &after, Point(0, 500)).unwrap();
    assert_eq!(a, [357, 156]);
    assert!((b[0] - a[0]).abs() <= 2);
    assert!((b[1] - a[1] + 128).abs() <= 3, "{a:?} -> {b:?}");
    let unknown = read_image(&fixtures.join("english-home.png")).unwrap();
    assert!(battlefield::locate(&fields, Screen::HomeScout, &unknown, Point(0, 500)).is_err());
    assert!(battlefield::locate(&fields, Screen::BuilderScout, &before, Point(0, 500)).is_err());
    assert!(battlefield::visible([640, -100]).is_none());
    assert!(battlefield::visible([110, 100]).is_none());
    for file in [
        "battlefield-builder-cave.png",
        "battlefield-builder-clearing.png",
    ] {
        let frame = read_image(&fixtures.join(file)).unwrap();
        let a = battlefield::locate(&fields, Screen::BuilderScout, &frame, Point(0, 500)).unwrap();
        let b = battlefield::locate(&fields, Screen::BuilderStage2, &frame, Point(0, 500)).unwrap();
        assert_eq!(a, b);
        assert!(battlefield::identify(&fields, Screen::HomeScout, &frame).is_err());
    }
    let cave = read_image(&fixtures.join("battlefield-builder-cave.png")).unwrap();
    let moved = read_image(&fixtures.join("battlefield-builder-cave-panned.png")).unwrap();
    let a = battlefield::locate(&fields, Screen::BuilderScout, &cave, Point(500, 1000)).unwrap();
    let b = battlefield::locate(&fields, Screen::BuilderScout, &moved, Point(500, 1000)).unwrap();
    assert!((a[0] - b[0]).abs() <= 3);
    assert!((a[1] - b[1] - 163).abs() <= 3);
    let zoomed = read_image(&fixtures.join("battlefield-builder-clearing-zoomed.png")).unwrap();
    let field = &fields[battlefield::identify(&fields, Screen::BuilderScout, &zoomed).unwrap()];
    let (scale, _) = field.transform(&zoomed, None).unwrap().unwrap();
    assert!((1.04..=1.06).contains(&scale));
    let panned = read_image(&fixtures.join("battlefield-builder-clearing-panned.png")).unwrap();
    let i = battlefield::identify(&fields, Screen::BuilderScout, &panned).unwrap();
    let transform = fields[i].transform(&panned, None).unwrap().unwrap();
    assert!((1.02..=1.05).contains(&transform.0));
    assert!((-139.0..=-132.0).contains(&transform.1[1]));
}

#[test]
fn missing_conflicting_and_duplicate_landmarks_cannot_authorize_positions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut fields = battlefield::load(&root).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/battlefield-classic.png");
    let source = read_image(&fixture).unwrap();
    let index = battlefield::identify(&fields, Screen::HomeScout, &source).unwrap();
    let field = &fields[index];
    let transform = field.transform(&source, None).unwrap().unwrap();
    assert!(field.transform(&source, Some(transform)).unwrap().is_some());
    let mut missing =
        Mat::new_rows_cols_with_default(720, 1280, opencv::core::CV_8UC3, Scalar::all(0.)).unwrap();
    for a in field.anchors.iter().take(2) {
        let template = read_image(&root.join(&a.target.file)).unwrap();
        let r = Rect::new(
            a.reference.0 as i32 - template.cols() / 2,
            a.reference.1 as i32 - template.rows() / 2,
            template.cols(),
            template.rows(),
        );
        Mat::roi(&source, r)
            .unwrap()
            .copy_to(&mut Mat::roi_mut(&mut missing, r).unwrap())
            .unwrap();
    }
    assert!(field.transform(&missing, None).unwrap().is_none());
    let mut conflict = source.clone();
    let a = &field.anchors[0];
    let template = read_image(&root.join(&a.target.file)).unwrap();
    let r = Rect::new(
        a.reference.0 as i32 - template.cols() / 2,
        a.reference.1 as i32 - template.rows() / 2,
        template.cols(),
        template.rows(),
    );
    Mat::roi_mut(&mut conflict, r)
        .unwrap()
        .set_to(&Scalar::all(0.), &opencv::core::no_array())
        .unwrap();
    template
        .copy_to(
            &mut Mat::roi_mut(&mut conflict, Rect::new(r.x + 50, r.y, r.width, r.height)).unwrap(),
        )
        .unwrap();
    assert!(field.transform(&conflict, None).unwrap().is_none());
    let duplicate = battlefield::load(&root).unwrap().remove(index);
    fields.push(duplicate);
    assert!(battlefield::identify(&fields, Screen::HomeScout, &source).is_err());
}

#[test]
fn captured_zoom_and_pan_keep_the_same_outer_border() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let fields = battlefield::load(&root).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for (file, scale_range) in [
        ("battlefield-forest-small.png", 0.91..=0.95),
        ("battlefield-forest-panned.png", 0.99..=1.03),
    ] {
        let frame = read_image(&fixtures.join(file)).unwrap();
        let index = battlefield::identify(&fields, Screen::HomeScout, &frame).unwrap();
        let field = &fields[index];
        assert_eq!(field.name, "forest");
        let transform = field.transform(&frame, None).unwrap().unwrap();
        assert!(scale_range.contains(&transform.0));
        let tracked = field.transform(&frame, Some(transform)).unwrap().unwrap();
        for point in [
            Point(0, 500),
            Point(1000, 500),
            Point(500, 0),
            Point(500, 1000),
        ] {
            let a = field.project(point, transform);
            let b = field.project(point, tracked);
            assert!((a[0] - b[0]).abs() <= 2 && (a[1] - b[1]).abs() <= 2);
        }
    }
}

#[test]
fn deployment_fixtures_only_retain_documented_regions() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures.join("deployment.json")).unwrap()).unwrap();
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
            .set_to(&Scalar::all(0.), &opencv::core::no_array())
            .unwrap();
        }
        assert!(
            frame.data_bytes().unwrap().iter().all(|v| *v == 0),
            "{file}"
        );
    }
}
