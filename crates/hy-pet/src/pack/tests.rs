//! Pet-pack tests: the shipped packs (build-time check), and the manifest rules.

use std::path::PathBuf;

use super::*;

fn packs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs")
}

fn turtle_json() -> serde_json::Value {
    serde_json::from_str(BUILTIN_SOURCES[0].1).expect("turtle json")
}

fn turtle_art() -> &'static str {
    BUILTIN_SOURCES[0].2
}

fn parse_value(v: &serde_json::Value) -> Result<Pack, PackError> {
    parse_pack("turtle", &v.to_string(), turtle_art())
}

fn refused_with(v: &serde_json::Value, needle: &str) {
    let e = parse_value(v).expect_err("should be refused");
    assert!(
        e.to_string().contains(needle),
        "{e} should mention {needle:?}"
    );
}

#[test]
fn all_ten_builtin_packs_load_four_drawn_six_pending() {
    let (lib, refused) = Library::builtin().expect("library");
    assert!(refused.is_empty(), "refused: {refused:?}");
    assert_eq!(lib.packs().len(), 10);
    let ids: Vec<&str> = lib.packs().iter().map(Pack::id).collect();
    assert_eq!(ids, BUILTIN_IDS);
    let drawn: Vec<&str> = lib
        .packs()
        .iter()
        .filter(|p| p.selectable())
        .map(Pack::id)
        .collect();
    assert_eq!(drawn, ["turtle", "cat", "drop", "owl"]);
    let pending = lib
        .packs()
        .iter()
        .filter(|p| p.manifest.art_status == ArtStatus::PendingReview)
        .count();
    assert_eq!(pending, 6);
    assert_eq!(lib.default_pack().id(), "turtle");
    assert_eq!(lib.default_pack().manifest.default_name, "Shelly");
}

#[test]
fn every_pack_directory_is_built_in_and_valid() {
    // The build-time check: what is on disk is exactly what is compiled in.
    let (lib, refused) = Library::load_dir(&packs_dir()).expect("packs dir");
    assert!(refused.is_empty(), "refused: {refused:?}");
    let mut on_disk: Vec<&str> = lib.packs().iter().map(Pack::id).collect();
    on_disk.sort_unstable();
    let mut built_in = BUILTIN_IDS.to_vec();
    built_in.sort_unstable();
    assert_eq!(on_disk, built_in);
    for (dir, json, svg) in BUILTIN_SOURCES {
        let disk = lib.get(dir).expect("on disk");
        assert_eq!(
            disk.manifest,
            parse_pack(dir, json, svg).expect("ok").manifest
        );
    }
}

#[test]
fn pending_packs_are_labelled_and_not_selectable() {
    let (lib, _) = Library::builtin().expect("library");
    for p in lib
        .packs()
        .iter()
        .filter(|p| p.manifest.art_status == ArtStatus::PendingReview)
    {
        assert!(!p.selectable());
        assert!(lib.selectable(p.id()).is_none());
        assert!(
            p.manifest.provenance.review.contains("Art pending review"),
            "{} must say art pending review",
            p.id()
        );
        // No invented art: the placeholder is an outline and a letter only.
        let report = check_art(&p.art_svg).expect("art");
        let elements: Vec<&str> = report.elements.iter().map(String::as_str).collect();
        assert_eq!(elements, ["rect", "svg", "text"], "{}", p.id());
    }
}

#[test]
fn drawn_packs_record_provenance_and_three_colours() {
    let (lib, _) = Library::builtin().expect("library");
    for p in lib.packs().iter().filter(|p| p.selectable()) {
        assert_eq!(p.manifest.palette.len(), 3);
        assert!(p.manifest.provenance.origin.contains("prototype"));
        assert!(p.manifest.provenance.licence.contains("no third-party"));
        assert_eq!(p.colours(2).name, p.manifest.palette[2].name);
        assert_eq!(p.colours(9).name, p.manifest.palette[0].name, "fallback");
    }
}

#[test]
fn library_skips_a_bad_pack_and_reports_it() {
    let mut sources: Vec<(&str, &str, &str)> = BUILTIN_SOURCES.to_vec();
    sources[1].2 = r#"<svg viewBox="0 0 128 128"><script/></svg>"#;
    let (lib, refused) = Library::from_sources(sources).expect("still usable");
    assert_eq!(lib.packs().len(), 9);
    assert!(lib.get("cat").is_none());
    assert_eq!(refused.len(), 1);
    assert!(matches!(refused[0], PackError::Art { .. }));
}

#[test]
fn library_refuses_repeated_ids_and_needs_a_selectable_pet() {
    let t = BUILTIN_SOURCES[0];
    let (lib, refused) = Library::from_sources([t, t]).expect("usable");
    assert_eq!(lib.packs().len(), 1);
    assert_eq!(refused.len(), 1);
    let only_pending = BUILTIN_SOURCES
        .iter()
        .copied()
        .filter(|(d, _, _)| !["turtle", "cat", "drop", "owl"].contains(d));
    assert!(Library::from_sources(only_pending).is_err());
    assert!(Library::from_sources([]).is_err());
}

#[test]
fn manifest_rules() {
    let base = turtle_json();
    parse_value(&base).expect("base is valid");

    let mut v = base.clone();
    v["format"] = 2.into();
    refused_with(&v, "format 2");

    let mut v = base.clone();
    v["id"] = "cat".into();
    refused_with(&v, "does not match");

    let mut v = base.clone();
    v["id"] = "Turtle".into();
    refused_with(&v, "lowercase");

    let mut v = base.clone();
    v["version"] = "1.0".into();
    refused_with(&v, "major.minor.patch");

    let mut v = base.clone();
    v["default_name"] = "A name far longer than twenty".into();
    refused_with(&v, "default_name");

    let mut v = base.clone();
    v["default_name"] = "   ".into();
    refused_with(&v, "default_name");

    let mut v = base.clone();
    v["palette"].as_array_mut().expect("array").pop();
    refused_with(&v, "exactly 3");

    let mut v = base.clone();
    v["palette"][0]["a"] = "green".into();
    refused_with(&v, "#rrggbb");

    let mut v = base.clone();
    v["palette"][1]["name"] = v["palette"][0]["name"].clone();
    refused_with(&v, "repeated");

    let mut v = base.clone();
    v["hitbox"]["width"] = 200.into();
    refused_with(&v, "hitbox");

    let mut v = base.clone();
    v["art"] = "../../secret.svg".into();
    refused_with(&v, "art.svg");

    let mut v = base.clone();
    v["provenance"]["licence"] = "".into();
    refused_with(&v, "licence");
}

#[test]
fn manifest_shape_is_strict() {
    let base = turtle_json();

    let mut v = base.clone();
    v["script"] = "alert(1)".into();
    refused_with(&v, "unknown field");

    let mut v = base.clone();
    v["states"]
        .as_object_mut()
        .expect("object")
        .remove("disconnected");
    refused_with(&v, "missing field `disconnected`");

    let mut v = base.clone();
    v["states"]["meditating"] = v["states"]["idle"].clone();
    refused_with(&v, "unknown field");

    let mut v = base.clone();
    v["states"]["idle"]["animation"] = "spin".into();
    refused_with(&v, "unknown variant");

    let mut v = base.clone();
    v["default_tone"] = "sarcastic".into();
    refused_with(&v, "unknown PetTone");

    let mut v = base.clone();
    v["art_status"] = "final".into();
    refused_with(&v, "unknown variant");

    assert!(matches!(
        parse_pack("turtle", "not json", turtle_art()),
        Err(PackError::Json { .. })
    ));
}

#[test]
fn drawn_art_must_use_the_palette_and_mark_its_eyes() {
    let json = BUILTIN_SOURCES[0].1;
    let no_palette = r##"<svg viewBox="0 0 128 128"><ellipse class="eye" cx="1" cy="1" rx="1" ry="1" fill="#000"/></svg>"##;
    let e = parse_pack("turtle", json, no_palette).expect_err("refused");
    assert!(e.to_string().contains("hy-a"));
    let no_eyes = r#"<svg viewBox="0 0 128 128"><circle class="hy-a" cx="1" cy="1" r="1"/></svg>"#;
    let e = parse_pack("turtle", json, no_eyes).expect_err("refused");
    assert!(e.to_string().contains("eye"));
}

#[test]
fn load_dir_reports_missing_files() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (dir, json, svg) = BUILTIN_SOURCES[0];
    std::fs::create_dir(tmp.path().join(dir)).expect("mkdir");
    std::fs::write(tmp.path().join(dir).join("pet.json"), json).expect("write");
    std::fs::write(tmp.path().join(dir).join("art.svg"), svg).expect("write");
    std::fs::create_dir(tmp.path().join("broken")).expect("mkdir");
    std::fs::write(tmp.path().join("broken").join("pet.json"), "{}").expect("write");
    let (lib, refused) = Library::load_dir(tmp.path()).expect("usable");
    assert_eq!(lib.packs().len(), 1);
    assert_eq!(refused.len(), 1);
    assert!(matches!(refused[0], PackError::Io { .. }));
    assert!(Library::load_dir(&tmp.path().join("nope")).is_err());
}

#[test]
fn pack_serialises_flat_for_the_pages() {
    let (lib, _) = Library::builtin().expect("library");
    let v = serde_json::to_value(lib.default_pack()).expect("json");
    assert_eq!(v["id"], "turtle");
    assert_eq!(v["art_status"], "drawn");
    assert!(v["art_svg"].as_str().expect("svg").starts_with("<svg"));
}
