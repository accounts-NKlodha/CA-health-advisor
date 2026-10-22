//! Pet edits through the real `hy-core` store: persistence, size rules, round trips and
//! restart (reopening the database file).

use std::sync::Arc;

use hy_core::clock::SystemClock;

use super::*;
use crate::hotkey::HOTKEY_CHOICES;

fn lib() -> Library {
    Library::builtin().expect("library").0
}

fn store() -> Store {
    Store::open_in_memory(Arc::new(SystemClock::new())).expect("store")
}

fn edit(store: &Store, lib: &Library, e: PetEdit) -> PetProfile {
    apply_edit(store, lib, &e).expect("edit accepted")
}

fn refused(store: &Store, lib: &Library, e: PetEdit) -> String {
    match apply_edit(store, lib, &e) {
        Err(EditError::Invalid(m)) => m,
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[test]
fn defaults_before_anything_is_saved() {
    let (s, l) = (store(), lib());
    let p = current_profile(&s, &l).expect("profile");
    assert_eq!(p.pet_id, "turtle");
    assert_eq!(p.display_name, "Shelly");
    assert_eq!(p.scale_pct, 100);
    assert_eq!(p.edge, PetEdge::Right);
    assert!(p.motion);
    assert_eq!(p.position, None);
    assert!(
        !has_saved_profile(&s).expect("read"),
        "nothing saved until an edit"
    );
}

#[test]
fn every_offered_size_is_saved_and_read_back() {
    let (s, l) = (store(), lib());
    for pct in (75..=150).step_by(5) {
        let p = edit(&s, &l, PetEdit::Size { pct });
        assert_eq!(p.scale_pct, pct);
        assert_eq!(current_profile(&s, &l).expect("read").scale_pct, pct);
    }
    assert!(has_saved_profile(&s).expect("read"));
}

#[test]
fn sizes_outside_the_range_or_off_step_are_refused_and_not_saved() {
    let (s, l) = (store(), lib());
    edit(&s, &l, PetEdit::Size { pct: 125 });
    for pct in [0, 70, 74, 76, 151, 155, 200, 101] {
        let m = refused(&s, &l, PetEdit::Size { pct });
        assert!(m.contains("75% to 150%"), "{m}");
        assert_eq!(current_profile(&s, &l).expect("read").scale_pct, 125);
    }
}

#[test]
fn size_survives_a_restart() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("hy.db");
    let l = lib();
    {
        let s = Store::open(&path, Arc::new(SystemClock::new())).expect("open");
        edit(&s, &l, PetEdit::Size { pct: 145 });
        edit(
            &s,
            &l,
            PetEdit::Rename {
                name: "Tortuga".into(),
            },
        );
        edit(&s, &l, PetEdit::Position { x: -1200, y: 340 });
        set_panel_hotkey(&s, Some("Ctrl+Alt+J".parse().expect("hotkey"))).expect("save");
    }
    let s = Store::open(&path, Arc::new(SystemClock::new())).expect("reopen");
    let p = current_profile(&s, &l).expect("read");
    assert_eq!(p.scale_pct, 145);
    assert_eq!(p.display_name, "Tortuga");
    assert_eq!(p.position, Some((-1200, 340)));
    assert_eq!(
        panel_hotkey(&s).expect("read").map(|h| h.to_string()),
        Some("Ctrl+Alt+J".to_owned())
    );
}

#[test]
fn every_edit_round_trips_through_the_store() {
    let (s, l) = (store(), lib());
    let edits = [
        PetEdit::ChoosePet {
            pet_id: "owl".into(),
        },
        PetEdit::Rename {
            name: "  Professor  ".into(),
        },
        PetEdit::Size { pct: 80 },
        PetEdit::Colour { variant: 2 },
        PetEdit::Edge {
            edge: PetEdge::Free,
        },
        PetEdit::Motion { on: false },
        PetEdit::Tone {
            tone: PetTone::Cheerful,
        },
        PetEdit::Position { x: 10, y: 20 },
    ];
    for e in &edits {
        let saved = apply_edit(&s, &l, e).expect("ok");
        assert_eq!(current_profile(&s, &l).expect("read"), saved);
    }
    let p = current_profile(&s, &l).expect("read");
    assert_eq!(p.pet_id, "owl");
    assert_eq!(p.display_name, "Professor");
    assert_eq!(p.scale_pct, 80);
    assert_eq!(p.colour_variant, 2);
    assert_eq!(p.edge, PetEdge::Free);
    assert!(!p.motion);
    assert_eq!(p.tone, PetTone::Cheerful);
    assert_eq!(p.position, Some((10, 20)));
    let p = edit(&s, &l, PetEdit::ClearPosition);
    assert_eq!(p.position, None);
}

#[test]
fn edits_arrive_as_json_from_the_pages() {
    let (s, l) = (store(), lib());
    for (json, check) in [
        (r#"{"kind":"size","pct":150}"#, "scale"),
        (r#"{"kind":"choose_pet","pet_id":"cat"}"#, "pet"),
        (r#"{"kind":"edge","edge":"left"}"#, "edge"),
        (r#"{"kind":"tone","tone":"plain"}"#, "tone"),
        (r#"{"kind":"reset_defaults"}"#, "reset"),
    ] {
        let e: PetEdit = serde_json::from_str(json).expect(check);
        apply_edit(&s, &l, &e).expect(check);
    }
    assert!(serde_json::from_str::<PetEdit>(r#"{"kind":"size","pct":100,"x":1}"#).is_err());
    assert!(serde_json::from_str::<PetEdit>(r#"{"kind":"explode"}"#).is_err());
}

#[test]
fn pending_and_unknown_pets_cannot_be_chosen() {
    let (s, l) = (store(), lib());
    for id in ["panda", "dog", "rabbit", "elephant", "koala", "flamingo"] {
        let m = refused(&s, &l, PetEdit::ChoosePet { pet_id: id.into() });
        assert!(m.contains("pending review"), "{m}");
    }
    let m = refused(
        &s,
        &l,
        PetEdit::ChoosePet {
            pet_id: "dragon".into(),
        },
    );
    assert!(m.contains("not in the library"));
    assert_eq!(current_profile(&s, &l).expect("read").pet_id, "turtle");
}

#[test]
fn unchanged_name_follows_the_pet_and_a_chosen_name_is_kept() {
    let (s, l) = (store(), lib());
    let p = edit(
        &s,
        &l,
        PetEdit::ChoosePet {
            pet_id: "cat".into(),
        },
    );
    assert_eq!(p.display_name, "Miso");
    edit(
        &s,
        &l,
        PetEdit::Rename {
            name: "Biscuit".into(),
        },
    );
    let p = edit(
        &s,
        &l,
        PetEdit::ChoosePet {
            pet_id: "drop".into(),
        },
    );
    assert_eq!(p.display_name, "Biscuit");
}

#[test]
fn names_are_checked() {
    let (s, l) = (store(), lib());
    for bad in ["", "   ", "abcdefghijklmnopqrstu", "tab\there"] {
        let m = refused(&s, &l, PetEdit::Rename { name: bad.into() });
        assert!(m.contains("1 to 20"), "{m}");
    }
    // Twenty characters of any script are fine.
    let p = edit(
        &s,
        &l,
        PetEdit::Rename {
            name: "कछुआ कछुआ कछुआ कछुआ".into(),
        },
    );
    assert_eq!(p.display_name.chars().count(), 19);
}

#[test]
fn colour_variants_are_0_to_2() {
    let (s, l) = (store(), lib());
    edit(&s, &l, PetEdit::Colour { variant: 1 });
    let m = refused(&s, &l, PetEdit::Colour { variant: 3 });
    assert!(m.contains("three colours"));
}

#[test]
fn reset_restores_defaults_but_keeps_the_pet() {
    let (s, l) = (store(), lib());
    edit(
        &s,
        &l,
        PetEdit::ChoosePet {
            pet_id: "owl".into(),
        },
    );
    edit(&s, &l, PetEdit::Size { pct: 150 });
    edit(
        &s,
        &l,
        PetEdit::Rename {
            name: "Night".into(),
        },
    );
    edit(&s, &l, PetEdit::Motion { on: false });
    edit(&s, &l, PetEdit::Position { x: 5, y: 5 });
    let p = edit(&s, &l, PetEdit::ResetDefaults);
    assert_eq!(p.pet_id, "owl");
    assert_eq!(p.display_name, "Hoot");
    assert_eq!(p.scale_pct, 100);
    assert!(p.motion);
    assert_eq!(p.position, None, "goes home above the clock");
    assert_eq!(p.edge, PetEdge::Right);
}

#[test]
fn a_saved_pet_that_is_no_longer_selectable_falls_back() {
    let (s, l) = (store(), lib());
    // As if an older build had saved a pet whose pack is now missing.
    let mut p = PetProfile::defaults("dragon", "Smaug");
    p.scale_pct = 130;
    s.save_pet_profile(&p).expect("save");
    let cur = current_profile(&s, &l).expect("read");
    assert_eq!(cur.pet_id, "turtle");
    assert_eq!(cur.display_name, "Smaug", "a custom name is kept");
    assert_eq!(cur.scale_pct, 130);
    // A pending pet saved directly (not through apply_edit) also falls back, and its
    // default name follows.
    s.save_pet_profile(&PetProfile::defaults("panda", "Panda"))
        .expect("save");
    let cur = current_profile(&s, &l).expect("read");
    assert_eq!(cur.pet_id, "turtle");
    assert_eq!(cur.display_name, "Shelly");
}

#[test]
fn hotkey_setting_defaults_turns_off_and_survives_bad_values() {
    let s = store();
    assert_eq!(
        panel_hotkey(&s)
            .expect("read")
            .map(|h| h.to_string())
            .as_deref(),
        Some("Ctrl+Alt+H")
    );
    set_panel_hotkey(&s, None).expect("save");
    assert_eq!(panel_hotkey(&s).expect("read"), None);
    for c in HOTKEY_CHOICES {
        set_panel_hotkey(&s, Some(c.parse().expect("choice"))).expect("save");
        assert_eq!(
            panel_hotkey(&s)
                .expect("read")
                .map(|h| h.to_string())
                .as_deref(),
            Some(c)
        );
    }
    s.put_setting(keys::PANEL_HOTKEY, "H").expect("raw");
    assert_eq!(
        panel_hotkey(&s)
            .expect("read")
            .map(|h| h.to_string())
            .as_deref(),
        Some("Ctrl+Alt+H"),
        "an unusable saved value falls back to the default"
    );
    s.put_setting(keys::PANEL_HOTKEY, &42).expect("raw");
    assert!(panel_hotkey(&s).expect("read").is_some());
}
