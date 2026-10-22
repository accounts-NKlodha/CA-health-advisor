//! Routing the user's answers back to the coaching engine: actions in the bubble, the pet
//! menu, the keyboard panel and the tray, the bubble's close button, clicks on the pet
//! and the tray's "Show pet".
//!
//! Routing is pure: it reads only the event and a small [`RouteState`] snapshot that the
//! link updates before it changes the shell. It runs on the UI thread inside
//! [`crate::shell::ShellEventSink::on_event`], which must not block, so it never touches
//! the engine or the store; the resulting [`Command`] is applied by the link afterwards.

use chrono::{NaiveDate, Weekday};
use hy_core::coach::{
    CheckInAnswer, GuideAction, Input, PauseLength, PomodoroPreset, RecoveryAction, Reflection,
    Response,
};
use hy_core::schema::{RoutineOutcome, RoutineVariant};

use super::present::{BubbleKind, WATER_SERVING_ML, action, day_key};
use crate::shell::{CloseCause, Handled, ShellEvent};

/// Something for the link to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Apply an engine input.
    Coach(Input),
    /// Log water from the pet menu (not an answer to a reminder), then re-evaluate.
    LogWater {
        /// Amount in ml.
        ml: u32,
    },
}

/// What the link last drew, as far as routing needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteState {
    /// The id of the engine bubble the link last showed, if it is still meant to be open.
    pub bubble_id: Option<String>,
    /// A click on the pet reopens a waiting reminder or a pending Pomodoro card.
    pub click_reopens: bool,
}

/// The outcome of routing one shell event.
#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    /// Whether the shell should skip its default (the pet menu after a click).
    pub handled: Handled,
    /// What the link should do, if anything.
    pub command: Option<Command>,
}

impl Routed {
    const fn none() -> Self {
        Self {
            handled: Handled::No,
            command: None,
        }
    }

    const fn coach(input: Input) -> Self {
        Self {
            handled: Handled::Yes,
            command: Some(Command::Coach(input)),
        }
    }
}

/// Route one shell event.
#[must_use]
pub fn route(state: &RouteState, event: &ShellEvent) -> Routed {
    match event {
        ShellEvent::BubbleAction {
            bubble_id,
            action_id,
        } => {
            // Only the bubble the link last showed; an older copy is stale.
            if state.bubble_id.as_deref() != Some(bubble_id.as_str()) {
                return Routed::none();
            }
            BubbleKind::parse(bubble_id)
                .and_then(|k| bubble_command(k, action_id))
                .map_or_else(Routed::none, |c| Routed {
                    handled: Handled::Yes,
                    command: Some(c),
                })
        }
        ShellEvent::BubbleClosed { bubble_id, cause } => match cause {
            // The link replaced its own bubble: nothing for the engine.
            CloseCause::Replaced => Routed::none(),
            // The close button, a click on the pet, or the user hiding the pet: the engine
            // collapses a reminder to its waiting badge (no answer is recorded). A bubble
            // the engine already withdrew (a presentation) no longer matches.
            CloseCause::User | CloseCause::Hidden => {
                if state.bubble_id.as_deref() == Some(bubble_id.as_str()) {
                    Routed::coach(Input::CloseBubble)
                } else {
                    Routed::none()
                }
            }
        },
        ShellEvent::MenuAction { action_id } => {
            menu_command(action_id).map_or_else(Routed::none, |c| Routed {
                handled: Handled::Yes,
                command: Some(c),
            })
        }
        ShellEvent::PetClicked { .. } => {
            if state.click_reopens {
                Routed::coach(Input::OpenPending)
            } else {
                Routed::none()
            }
        }
        ShellEvent::PetShownByUser => Routed::coach(Input::ShowPet),
    }
}

/// The engine input for an action in a bubble of this kind.
#[must_use]
pub fn bubble_command(kind: BubbleKind, action_id: &str) -> Option<Command> {
    use action as a;
    let input = match kind {
        BubbleKind::Reminder { offer } => {
            let response = match action_id {
                a::DONE => Response::Done,
                a::START => Response::Start,
                a::SNOOZE => Response::Snooze,
                a::DISMISS => Response::Dismiss,
                a::LOG_WATER => Response::LogWater {
                    ml: WATER_SERVING_ML,
                },
                _ => return None,
            };
            Input::Respond { offer, response }
        }
        BubbleKind::PomodoroBreak => match action_id {
            a::START_BREAK => Input::StartPomodoroBreak,
            a::SKIP_BREAK => Input::SkipPomodoroBreak,
            a::END_POMODORO => Input::EndPomodoro,
            _ => return None,
        },
        BubbleKind::PomodoroRoundDone => match action_id {
            a::NEXT_ROUND => Input::NextPomodoroRound,
            a::END_POMODORO => Input::EndPomodoro,
            _ => return None,
        },
        BubbleKind::PomodoroPaused => match action_id {
            a::RESUME_POMODORO => Input::ResumePomodoro,
            a::END_POMODORO => Input::EndPomodoro,
            _ => return None,
        },
        BubbleKind::CheckIn => Input::CheckIn(match action_id {
            a::CHECK_IN_YES => CheckInAnswer::Yes,
            a::CHECK_IN_NO => CheckInAnswer::No,
            a::CHECK_IN_NOT_TODAY => CheckInAnswer::DontAskToday,
            _ => return None,
        }),
        BubbleKind::Lighter => Input::LighterSchedule {
            accept: match action_id {
                a::USE_LIGHT => true,
                a::KEEP_INTENSITY => false,
                _ => return None,
            },
        },
        BubbleKind::MovementGuide => Input::Guide(match action_id {
            a::SEATED => GuideAction::Variant(RoutineVariant::Seated),
            a::STANDING => GuideAction::Variant(RoutineVariant::Standing),
            a::STEP_DONE => GuideAction::Done,
            a::STEP_SKIP => GuideAction::Skip,
            a::STOP => GuideAction::Stop,
            a::LATER => GuideAction::Later,
            a::SKIP_TODAY => GuideAction::SkipToday,
            a::OTHER_DAY => GuideAction::OtherDay,
            a::BACK => GuideAction::Back,
            a::OK => GuideAction::Close,
            other => GuideAction::Day(parse_day(other.strip_prefix(a::DAY_PREFIX)?)?),
        }),
        BubbleKind::DeadlineDay => Input::CheckIn(match action_id {
            a::CHECK_IN_YES => CheckInAnswer::Yes,
            a::CHECK_IN_NO => CheckInAnswer::No,
            _ => return None,
        }),
        BubbleKind::RecoveryInvite | BubbleKind::RecoveryDates => {
            Input::Recovery(match action_id {
                a::CHOOSE_DATE if kind == BubbleKind::RecoveryInvite => RecoveryAction::ChooseDate,
                a::LATER => RecoveryAction::Later,
                a::NO_THANKS if kind == BubbleKind::RecoveryInvite => RecoveryAction::NoThanks,
                other => RecoveryAction::PlanFor(parse_date(other.strip_prefix(a::PLAN_PREFIX)?)?),
            })
        }
        BubbleKind::RecoveryPlanned => Input::Recovery(match action_id {
            a::OK => RecoveryAction::Ok,
            a::CHOOSE_DATE => RecoveryAction::ChooseDate,
            a::CANCEL_PLAN => RecoveryAction::CancelPlan,
            _ => return None,
        }),
        BubbleKind::RecoveryMorning => Input::Recovery(match action_id {
            a::OK => RecoveryAction::Ok,
            a::CANCEL_PLAN => RecoveryAction::CancelPlan,
            _ => return None,
        }),
        BubbleKind::RecoveryWalk => Input::Recovery(match action_id {
            a::WALK_NOW => RecoveryAction::WalkNow,
            a::WALK_NOT_NOW => RecoveryAction::WalkNotNow,
            _ => return None,
        }),
        BubbleKind::RecoveryReflection => {
            Input::Recovery(RecoveryAction::Reflect(match action_id {
                a::REFLECT_YES => Reflection::Yes,
                a::REFLECT_SOMEWHAT => Reflection::Somewhat,
                a::REFLECT_NO => Reflection::No,
                a::REFLECT_SKIP => Reflection::Skip,
                _ => return None,
            }))
        }
    };
    Some(Command::Coach(input))
}

/// The weekday named by a `day_` action suffix (Monday to Saturday).
fn parse_day(key: &str) -> Option<Weekday> {
    [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
    ]
    .into_iter()
    .find(|d| day_key(*d) == key)
}

/// The date named by a `plan_` action suffix (`2026-10-21`).
fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// Split `rest` into a number and the confirmation flags that follow it.
fn number_and_flags<'a>(rest: &'a str, flags: &[&str]) -> Option<(u32, Vec<&'a str>)> {
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let n = rest[..digits].parse().ok()?;
    let mut tail = &rest[digits..];
    let mut found = Vec::new();
    while !tail.is_empty() {
        let f = flags.iter().find(|f| tail.starts_with(**f))?;
        let (head, more) = tail.split_at(f.len());
        if found.contains(&head) {
            return None;
        }
        found.push(head);
        tail = more;
    }
    Some((n, found))
}

/// The command for an engine entry in the pet menu, keyboard panel or tray.
#[must_use]
pub fn menu_command(id: &str) -> Option<Command> {
    use action as a;
    let coach = |i| Some(Command::Coach(i));
    match id {
        a::OPEN_PENDING => coach(Input::OpenPending),
        a::WATER => Some(Command::LogWater {
            ml: WATER_SERVING_ML,
        }),
        a::MOVEMENT_START => coach(Input::RoutineStarted),
        a::MOVEMENT_DONE | a::MOVEMENT_PARTIAL | a::MOVEMENT_STOP => {
            coach(Input::RoutineFinished {
                offer: None,
                outcome: match id {
                    a::MOVEMENT_DONE => RoutineOutcome::Done,
                    a::MOVEMENT_PARTIAL => RoutineOutcome::Partial,
                    _ => RoutineOutcome::None,
                },
            })
        }
        a::DEADLINE_DAY => coach(Input::DeadlineDay),
        a::RECOVERY_SHOW => coach(Input::Recovery(RecoveryAction::ShowPlan)),
        a::BREAK => coach(Input::MarkBreak),
        a::FOCUS_END => coach(Input::EndFocus),
        a::POMODORO_PAUSE => coach(Input::PausePomodoro),
        a::POMODORO_RESUME => coach(Input::ResumePomodoro),
        a::POMODORO_END => coach(Input::EndPomodoro),
        a::MEETING_ON => coach(Input::SetMeeting(true)),
        a::MEETING_OFF => coach(Input::SetMeeting(false)),
        a::PAUSE_30 => coach(Input::Pause(PauseLength::Minutes(30))),
        a::PAUSE_60 => coach(Input::Pause(PauseLength::Minutes(60))),
        a::PAUSE_TODAY => coach(Input::Pause(PauseLength::RestOfToday)),
        a::PAUSE_UNTIL_RESUMED => coach(Input::Pause(PauseLength::UntilResumed)),
        a::RESUME_COACHING => coach(Input::ResumeCoaching),
        _ => {
            if let Some(rest) = id.strip_prefix(a::FOCUS_PREFIX) {
                let (minutes, flags) = number_and_flags(rest, &[a::REPLACES_POMODORO])?;
                if ![30, 60, 90].contains(&minutes) {
                    return None;
                }
                return coach(Input::StartFocus {
                    minutes,
                    replace_pomodoro: flags.contains(&a::REPLACES_POMODORO),
                });
            }
            if let Some(rest) = id.strip_prefix(a::POMODORO_PREFIX) {
                let (work, flags) =
                    number_and_flags(rest, &[a::REPLACES_FOCUS, a::RESUMES_COACHING])?;
                let preset = match work {
                    25 => PomodoroPreset::Classic,
                    50 => PomodoroPreset::Long,
                    _ => return None,
                };
                return coach(Input::StartPomodoro {
                    preset,
                    replace_focus: flags.contains(&a::REPLACES_FOCUS),
                    resume_coaching: flags.contains(&a::RESUMES_COACHING),
                });
            }
            None
        }
    }
}
