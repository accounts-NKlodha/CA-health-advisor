//! Shell rules: one bubble at a time, presentation hiding with nothing replayed, the
//! tray's "Show pet" override, and "Bring pet above clock" while hidden.

use super::*;

fn reminder(id: &str) -> Bubble {
    Bubble {
        id: id.into(),
        title: "Movement: 48 min of work since your last break".into(),
        lines: vec![],
        chips: vec![BubbleChip {
            label: "Movement".into(),
            status: "due".into(),
            highlighted: true,
        }],
        actions: vec![
            BubbleAction {
                id: "start".into(),
                label: "Start movement".into(),
                style: ActionStyle::Primary,
            },
            BubbleAction {
                id: "snooze".into(),
                label: "Snooze 10 min".into(),
                style: ActionStyle::Normal,
            },
        ],
        ..Bubble::default()
    }
}

fn emitted(fx: &[Effect]) -> Vec<&ShellEvent> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Emit(ev) => Some(ev),
            _ => None,
        })
        .collect()
}

#[test]
fn at_most_one_bubble_and_the_replaced_owner_is_told() {
    let mut m = ShellModel::new();
    let (o, fx) = m.show_bubble(reminder("a"));
    assert_eq!(o, BubbleOutcome::Shown);
    assert_eq!(fx, vec![Effect::Render]);
    let (o, fx) = m.show_bubble(reminder("b"));
    assert_eq!(o, BubbleOutcome::Shown);
    assert_eq!(
        emitted(&fx),
        [&ShellEvent::BubbleClosed {
            bubble_id: "a".into(),
            cause: CloseCause::Replaced
        }]
    );
    assert!(matches!(m.open(), Open::Bubble { bubble } if bubble.id == "b"));
}

#[test]
fn presentation_hides_pet_and_bubble_and_nothing_is_replayed() {
    let mut m = ShellModel::new();
    m.show_bubble(reminder("a"));
    let fx = m.hide(HideReason::Presentation);
    assert!(fx.contains(&Effect::HideWindow));
    assert!(fx.contains(&Effect::UpdateTray));
    assert_eq!(
        emitted(&fx),
        [&ShellEvent::BubbleClosed {
            bubble_id: "a".into(),
            cause: CloseCause::Hidden
        }]
    );
    assert!(!m.is_visible());
    assert_eq!(m.tray().toggle_label, "Show pet");
    assert_eq!(m.tray().tooltip, "HealthierYou - pet hidden: presentation");
    // A reminder during the presentation is suppressed, not queued.
    let (o, fx) = m.show_bubble(reminder("b"));
    assert_eq!(o, BubbleOutcome::Suppressed);
    assert!(fx.is_empty());
    // The presentation ends: the pet returns quietly with nothing open.
    let fx = m.unhide(HideReason::Presentation);
    assert_eq!(
        fx,
        vec![Effect::ShowWindow, Effect::Render, Effect::UpdateTray]
    );
    assert_eq!(m.open(), &Open::None);
    assert_eq!(m.tray().toggle_label, "Hide pet");
}

#[test]
fn tray_show_pet_overrides_every_reason() {
    let mut m = ShellModel::new();
    m.hide(HideReason::User);
    m.hide(HideReason::Presentation);
    // Removing one reason is not enough.
    assert_eq!(m.unhide(HideReason::User), vec![Effect::UpdateTray]);
    assert!(!m.is_visible());
    m.hide(HideReason::User);
    let fx = m.user_show();
    assert!(fx.contains(&Effect::ShowWindow));
    // The engine hid the pet for the presentation, so it is told about the override.
    assert!(fx.contains(&Effect::Emit(ShellEvent::PetShownByUser)));
    assert!(m.is_visible());
    // The presentation ending later changes nothing.
    assert!(m.unhide(HideReason::Presentation).is_empty());
    assert!(m.user_show().is_empty());
    // Showing a pet the user hid tells the engine nothing.
    m.hide(HideReason::User);
    assert!(!m.user_show().iter().any(|e| matches!(e, Effect::Emit(_))));
}

#[test]
fn hiding_twice_or_unhiding_an_unset_reason_does_nothing() {
    let mut m = ShellModel::new();
    assert!(m.unhide(HideReason::Presentation).is_empty());
    assert!(!m.hide(HideReason::User).is_empty());
    assert!(m.hide(HideReason::User).is_empty());
    // A second reason while already hidden only refreshes the tray.
    assert_eq!(m.hide(HideReason::Presentation), vec![Effect::UpdateTray]);
}

#[test]
fn bring_back_shows_a_user_hidden_pet_but_not_during_a_presentation() {
    let mut m = ShellModel::new();
    m.hide(HideReason::User);
    let fx = m.bring_back();
    assert!(fx.contains(&Effect::ShowWindow));
    assert!(m.is_visible());

    let mut m = ShellModel::new();
    m.hide(HideReason::User);
    m.hide(HideReason::Presentation);
    let fx = m.bring_back();
    assert!(!fx.contains(&Effect::ShowWindow));
    assert!(!m.is_visible());
    assert_eq!(m.tray().tooltip, "HealthierYou - pet hidden: presentation");
    // When the presentation ends the pet comes back (the user's hide was cleared).
    assert!(
        m.unhide(HideReason::Presentation)
            .contains(&Effect::ShowWindow)
    );
}

#[test]
fn click_opens_the_menu_only_when_the_engine_does_not_handle_it() {
    let mut m = ShellModel::new();
    m.set_state(PetState::Waiting, None);
    let fx = m.pet_clicked();
    assert_eq!(
        emitted(&fx),
        [&ShellEvent::PetClicked {
            state: PetState::Waiting
        }]
    );
    assert_eq!(m.open(), &Open::None, "the companion decides after asking");
    assert_eq!(m.open_menu(), vec![Effect::Render]);
    assert_eq!(m.open(), &Open::Menu);
    // A second click closes the menu without telling the engine anything.
    let fx = m.pet_clicked();
    assert!(emitted(&fx).is_empty());
    assert_eq!(m.open(), &Open::None);
}

#[test]
fn click_on_an_open_bubble_closes_it_as_a_user_close() {
    let mut m = ShellModel::new();
    m.show_bubble(reminder("a"));
    let fx = m.pet_clicked();
    assert_eq!(
        emitted(&fx),
        [&ShellEvent::BubbleClosed {
            bubble_id: "a".into(),
            cause: CloseCause::User
        }]
    );
    assert!(m.user_close().is_empty(), "nothing left to close");
}

#[test]
fn menu_never_covers_the_engine_bubble_and_hidden_pets_have_no_menu() {
    let mut m = ShellModel::new();
    m.show_bubble(reminder("a"));
    assert!(m.open_menu().is_empty());
    let mut m = ShellModel::new();
    m.hide(HideReason::User);
    assert!(m.open_menu().is_empty());
}

#[test]
fn engine_bubble_replaces_the_menu_and_notices() {
    let mut m = ShellModel::new();
    m.open_menu();
    let (_, fx) = m.show_bubble(reminder("a"));
    assert!(emitted(&fx).is_empty(), "replacing the menu tells nobody");
    let mut m = ShellModel::new();
    m.show_notice(Bubble::notice("back", "Your pet is back above the clock."));
    m.show_bubble(reminder("a"));
    assert!(matches!(m.open(), Open::Bubble { .. }));
}

#[test]
fn notices_never_replace_the_engine_bubble_and_expire() {
    let mut m = ShellModel::new();
    m.show_bubble(reminder("a"));
    assert!(m.show_notice(Bubble::notice("back", "Back.")).is_empty());
    assert!(matches!(m.open(), Open::Bubble { .. }));
    let mut m = ShellModel::new();
    m.show_notice(Bubble::notice("back", "Back."));
    assert!(m.notice_expired("other").is_empty());
    assert_eq!(m.notice_expired("back"), vec![Effect::Render]);
    assert_eq!(m.open(), &Open::None);
    let mut hidden = ShellModel::new();
    hidden.hide(HideReason::Presentation);
    assert!(
        hidden
            .show_notice(Bubble::notice("back", "Back."))
            .is_empty()
    );
}

#[test]
fn actions_go_to_the_engine_only_for_the_open_bubble() {
    let mut m = ShellModel::new();
    m.show_bubble(reminder("a"));
    assert_eq!(
        emitted(&m.bubble_action("a", "snooze")),
        [&ShellEvent::BubbleAction {
            bubble_id: "a".into(),
            action_id: "snooze".into()
        }]
    );
    assert!(m.bubble_action("old", "snooze").is_empty(), "stale bubble");
    assert!(m.bubble_action("a", "launch").is_empty(), "unknown action");
    // The engine closes its own bubble without an echo.
    assert_eq!(m.close_bubble("a"), vec![Effect::Render]);
    assert!(m.close_bubble("a").is_empty());
}

#[test]
fn menu_actions_reach_the_engine_and_the_tray() {
    let mut m = ShellModel::new();
    let fx = m.set_menu_actions(vec![
        MenuAction {
            id: "water".into(),
            label: "+250 ml".into(),
            in_tray: false,
        },
        MenuAction {
            id: "pause".into(),
            label: "Pause...".into(),
            in_tray: true,
        },
    ]);
    assert!(fx.contains(&Effect::UpdateTray));
    assert_eq!(m.tray().actions.len(), 1);
    m.open_menu();
    let fx = m.menu_action("water");
    assert_eq!(
        emitted(&fx),
        [&ShellEvent::MenuAction {
            action_id: "water".into()
        }]
    );
    assert_eq!(m.open(), &Open::None, "the menu closes");
    assert!(m.menu_action("nope").is_empty());
}

#[test]
fn invalid_bubbles_are_refused() {
    let mut m = ShellModel::new();
    let mut b = reminder("a");
    b.actions.push(b.actions[0].clone());
    assert_eq!(m.show_bubble(b).0, BubbleOutcome::Invalid);
    let mut b = reminder("a");
    b.title = String::new();
    assert_eq!(m.show_bubble(b).0, BubbleOutcome::Invalid);
    let mut b = reminder("a");
    b.actions = (0..9)
        .map(|i| BubbleAction {
            id: format!("a{i}"),
            label: "x".into(),
            style: ActionStyle::Link,
        })
        .collect();
    assert_eq!(m.show_bubble(b).0, BubbleOutcome::Invalid);
    assert_eq!(m.open(), &Open::None);
}

#[test]
fn bubble_json_from_the_engine_is_strict() {
    let b: Bubble = serde_json::from_str(
        r#"{"id":"w1","title":"Water: 140 ml behind your own pace","actions":[{"id":"log","label":"Log 250 ml","style":"primary"}]}"#,
    )
    .expect("json");
    assert_eq!(b.actions[0].style, ActionStyle::Primary);
    assert!(serde_json::from_str::<Bubble>(r#"{"id":"x","title":"t","html":"<b>"}"#).is_err());
    let ev = serde_json::to_value(ShellEvent::PetClicked {
        state: PetState::Idle,
    })
    .expect("json");
    assert_eq!(
        ev,
        serde_json::json!({"kind": "pet_clicked", "state": "idle"})
    );
}

#[test]
fn state_changes_render() {
    let mut m = ShellModel::new();
    assert_eq!(
        m.set_state(PetState::Focus, Some("Focus 60 min".into())),
        vec![Effect::Render]
    );
    assert_eq!(m.state(), (PetState::Focus, Some("Focus 60 min")));
}
