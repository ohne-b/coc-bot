use coc_bot_core::{
    gameplay::profile::Profile,
    telemetry::Resource,
    vision::{Digits, read_image},
};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/python")
}
#[test]
fn english_wall_confirmations_require_matching_currency_and_reserves() {
    let profile = Profile::load(&root()).unwrap();
    let digits = Digits::load(&root(), Path::new("calibration/digits")).unwrap();
    for (village, file, resource) in [
        ("home", "english-wall_confirm_gold.png", Resource::Gold),
        ("home", "english-wall_confirm_elixir.png", Resource::Elixir),
        (
            "builder",
            "english-builder_wall_confirm_gold.png",
            Resource::Gold,
        ),
        (
            "builder",
            "english-builder_wall_confirm_elixir.png",
            Resource::Elixir,
        ),
    ] {
        let spec = &profile.walls[village];
        let image = read_image(&root().join("fixtures").join(file)).unwrap();
        let cost = spec
            .confirm_cost(&image, &digits, resource, u32::MAX, 0)
            .unwrap();
        assert!(cost > 0);
        assert_eq!(
            spec.confirm_cost(&image, &digits, resource, cost + 1_000_000, 1_000_000)
                .unwrap(),
            cost
        );
        assert!(
            spec.confirm_cost(&image, &digits, resource, cost + 999_999, 1_000_000)
                .is_err()
        );
        let other = if resource == Resource::Gold {
            Resource::Elixir
        } else {
            Resource::Gold
        };
        assert!(
            spec.confirm_cost(&image, &digits, other, u32::MAX, 0)
                .is_err()
        );
        assert!(
            spec.confirm_cost(&image, &digits, Resource::Dark, u32::MAX, 0)
                .is_err()
        );
    }
}
#[test]
fn village_target_and_wall_selection_are_loaded_from_english_assets() {
    let profile = Profile::load(&root()).unwrap();
    let home = read_image(&root().join("fixtures/english-home.png")).unwrap();
    let selected = read_image(&root().join("fixtures/english-wall_selected.png")).unwrap();
    assert!(profile.walls["home"].gem_shift(&home).is_ok());
    assert!(!profile.walls["home"].buttons(&selected).unwrap().is_empty());
}
