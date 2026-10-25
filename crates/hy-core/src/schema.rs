//! The `hy-core` data schema: the event envelope (architecture 6.1, PRD section 8), the
//! typed event payloads for V1 and V2, and the enumerations shared with the SQLite
//! `CHECK` constraints in `migrations/0001_initial.sql`.
//!
//! Every enumeration's text form is the value stored in the database. A test checks that
//! each `CHECK (... IN (...))` list in the migration matches the Rust enumeration.
//!
//! Event types live in Rust, not in a database `CHECK`, so a later slice can add a type
//! without a table rebuild. Payloads are validated here before they are written.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::time::UserZone;

/// Version of the event envelope and payload shapes written by this build.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Largest single water entry, in millilitres (wireframe B3: custom amount 1 to 2000 ml).
pub const MAX_WATER_ENTRY_ML: u32 = 2000;

/// Accepted range for the user's own daily water goal, in millilitres (D). The app never
/// suggests or raises the goal (WAT-01, WAT-02); this only rejects typing errors.
pub const WATER_GOAL_RANGE_ML: std::ops::RangeInclusive<u32> = 100..=10_000;

/// How far ahead of the clock an event may be stamped before it is refused (D).
pub const FUTURE_TOLERANCE: chrono::Duration = chrono::Duration::minutes(5);

macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$vmeta:meta])* $variant:ident => $text:literal ),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The stored text form.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(Error::invalid(format!(
                        concat!("unknown ", stringify!($name), " {:?}"),
                        other
                    ))),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let text = String::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }

        impl rusqlite::types::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.as_str().into())
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(v: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                let text = v.as_str()?;
                text.parse()
                    .map_err(|e: Error| rusqlite::types::FromSqlError::Other(Box::new(e)))
            }
        }
    };
}

text_enum! {
    /// Where an event came from (PRD section 8: manual, inferred and imported values stay
    /// distinguishable).
    EventSource {
        /// The user entered or confirmed it.
        Manual => "manual",
        /// Derived by the app (for example a quiet period from input idle).
        Inferred => "inferred",
        /// Reported by Windows (lock, unlock, sleep, resume).
        System => "system",
        /// Reserved for V3 wearable imports. The column accepts it; V1 and V2 refuse it.
        Imported => "imported",
    }
}

text_enum! {
    /// How trustworthy a value is.
    MeasurementStatus {
        /// The user said so; not verified.
        SelfReported => "self_reported",
        /// Directly observed by the PC (a lock event, a timer).
        Measured => "measured",
        /// An estimate under a documented rule (quiet time counted as work).
        Estimated => "estimated",
        /// No usable reading (for example the camera saw no one). Never a zero.
        NoReading => "no_reading",
    }
}

text_enum! {
    /// Optional sharing categories (architecture 6.2 and 13: a fixed list). All are off by
    /// default.
    Category {
        /// Hydration summary (ml, goal, number of entries).
        Hydration => "hydration",
        /// Reminder and exercise outcomes.
        Outcomes => "outcomes",
        /// Work-session summary, with no within-day timing.
        WorkSession => "work_session",
        /// Phone self-report counts by label.
        PhoneReports => "phone_reports",
        /// V2 camera summary.
        PostureSummary => "posture_summary",
    }
}

text_enum! {
    /// The three activities a reminder can offer, one at a time (wireframe B2).
    Activity {
        /// The movement routine.
        Movement => "movement",
        /// A 20-second eye rest.
        EyeRest => "eye_rest",
        /// A drink of water.
        Water => "water",
    }
}

text_enum! {
    /// The outcome of one offered activity (wireframe acceptance item 2.4). A response to
    /// one activity never changes another activity's status.
    ReminderOutcome {
        /// "Already did it" (self-reported).
        Done => "done",
        /// The user started the activity; its own record (a routine) holds the result.
        Started => "started",
        /// Snoozed; the activity returns once.
        Snoozed => "snoozed",
        /// Skipped for now.
        Skipped => "skipped",
        /// "Not now". Never counted as done.
        Dismissed => "dismissed",
        /// Collapsed to a badge after 10 minutes without a response.
        NoResponse => "no_response",
    }
}

text_enum! {
    /// Movement routine variant (MOV-01; wireframe B4).
    RoutineVariant {
        /// Seated alternative.
        Seated => "seated",
        /// Standing version.
        Standing => "standing",
    }
}

text_enum! {
    /// Result of a movement routine (wireframe 10.1, Codex source review 21 Oct).
    RoutineOutcome {
        /// Every step done.
        Done => "done",
        /// Some steps done; never counted as complete.
        Partial => "partial",
        /// Stopped with no step done ("Stopped, none recorded").
        None => "none",
        /// "Skip today".
        Skipped => "skipped",
    }
}

impl RoutineOutcome {
    /// Only done and partial reset the movement timer (wireframe 10.1).
    #[must_use]
    pub const fn resets_movement_timer(self) -> bool {
        matches!(self, Self::Done | Self::Partial)
    }
}

text_enum! {
    /// Result of one routine step.
    StepOutcome {
        /// Done.
        Done => "done",
        /// "Skip step". Never counted as done.
        Skipped => "skipped",
    }
}

text_enum! {
    /// Where a deadline-day check-in came from (wireframe B7).
    CheckInOrigin {
        /// Offered after a long day of estimated PC activity (optional, 9 h by default).
        LongDay => "long_day",
        /// The user chose "Deadline day" on the pet menu.
        Menu => "menu",
    }
}

text_enum! {
    /// The user's answer to a deadline-day check-in. Only the user decides; the app never
    /// infers stress or tiredness (wireframe B7).
    CheckInReply {
        /// "Yes, it was": today is marked as deadline-heavy.
        Yes => "yes",
        /// "No".
        No => "no",
        /// "Don't ask today".
        DontAskToday => "dont_ask_today",
    }
}

text_enum! {
    /// A change to the one pending recovery-day plan (wireframe W15, flow 4).
    PlanChange {
        /// Saved (a new plan, a new date or edited items). Replaces any earlier plan.
        Saved => "saved",
        /// Cancelled by the user.
        Cancelled => "cancelled",
    }
}

text_enum! {
    /// The end-of-day reflection on a recovery day: "Did this routine feel helpful?"
    /// It stays on this PC (architecture 6.2).
    ReflectionAnswer {
        /// Yes.
        Yes => "yes",
        /// Somewhat.
        Somewhat => "somewhat",
        /// No.
        No => "no",
    }
}

text_enum! {
    /// The meditation pause's version (V2-4; CAL-01): breath-focused, or eyes open with no
    /// attention on the breath.
    MeditationVersion {
        /// Breath-focused.
        Breath => "breath",
        /// Eyes open, attention on sounds, touch and the room; nothing about the breath.
        EyesOpen => "eyes_open",
    }
}

text_enum! {
    /// How a meditation pause ended. It stays on this PC.
    MeditationOutcome {
        /// Every part was reached and the time ran out (or Next on the last part).
        Done => "done",
        /// Stopped by the user, closed, or ended by a hold (a lock, meeting, presentation,
        /// focus, pause, the end of work hours) or the app closing.
        Stopped => "stopped",
    }
}

/// The meditation pause lengths offered, in minutes (2, 5 or 10; 5 is the default).
pub const MEDITATION_MINUTES: [u32; 3] = [2, 5, 10];
/// Parts in a meditation pause.
pub const MEDITATION_PARTS: u32 = 5;

text_enum! {
    /// Label of a phone self-report (DIS-01).
    PhoneLabel {
        /// No label chosen.
        Unlabelled => "unlabelled",
        /// Work-related.
        Work => "work",
        /// Personal.
        Personal => "personal",
        /// Unsure.
        Unsure => "unsure",
    }
}

text_enum! {
    /// Tracker session states (architecture 4.3).
    SessionState {
        /// Input within the last 60 s.
        Active => "active",
        /// No input for 1 to 15 minutes while unlocked and awake.
        Quiet => "quiet",
        /// No input for more than 15 minutes while unlocked.
        LongQuiet => "long_quiet",
        /// Lock or sleep of 2 minutes or more, or a break marked by the user.
        ConfirmedBreak => "confirmed_break",
        /// Lock or sleep under 2 minutes.
        ShortLock => "short_lock",
        /// Outside work hours.
        OffHours => "off_hours",
        /// The user paused coaching.
        Paused => "paused",
    }
}

text_enum! {
    /// Kinds of V2 posture observation (architecture 4.5).
    PostureKind {
        /// A sustained drift episode relative to calibration.
        DriftEpisode => "drift_episode",
        /// An interval with no usable reading.
        NoReading => "no_reading",
        /// An interval with a suspect or invalid baseline, or no usable calibration.
        BaselineSuspect => "baseline_suspect",
        /// A stretch of camera-on time (slice V2-3: the denominator of a day's coverage).
        Capture => "capture",
    }
}

text_enum! {
    /// Why a posture interval had no reading or a suspect baseline (architecture 4.5).
    PostureReason {
        /// No reading: nobody in view.
        Absent => "absent",
        /// No reading: a person, but not visible enough.
        LowConfidence => "low_confidence",
        /// No reading: a shoulder outside the frame.
        OutOfFrame => "out_of_frame",
        /// No reading: the pose worker stopped answering.
        Stalled => "stalled",
        /// Suspect: the app started with a stored calibration.
        AppRestart => "app_restart",
        /// Suspect: the camera stream started again (after a hold, sleep or a conflict).
        CameraRestart => "camera_restart",
        /// Suspect: a step change in where the shoulders sit in the frame or their scale.
        CameraMoved => "camera_moved",
        /// Suspect: the head-to-shoulder width ratio left its calibration band while
        /// posture stayed steady (the camera's viewpoint changed).
        ViewpointChanged => "viewpoint_changed",
        /// Not scored: no usable calibration (none yet, not usable, being taken, or
        /// invalid until the user recalibrates; slice V2-3).
        NoCalibration => "no_calibration",
    }
}

impl PostureReason {
    /// Whether the reason belongs to a no-reading interval (else to a suspect baseline).
    #[must_use]
    pub const fn is_no_reading(self) -> bool {
        matches!(
            self,
            Self::Absent | Self::LowConfidence | Self::OutOfFrame | Self::Stalled
        )
    }
}

text_enum! {
    /// What became of a drift episode's cue (architecture 4.5 quality report).
    CueOutcome {
        /// The pet showed the cue.
        Shown => "shown",
        /// Suppressed: within the cooldown after the last cue.
        Cooldown => "cooldown",
        /// Dropped: quiet focus was on.
        Focus => "focus",
        /// Suppressed: a meeting, presentation or other hold.
        Held => "held",
        /// Suppressed: another card, exercise or Pomodoro was on show, or the calm gap.
        Busy => "busy",
        /// The coaching engine did not answer.
        NoAnswer => "no_answer",
    }
}

text_enum! {
    /// Goals kept with effective-from dates (architecture 6.1 `goals_history`).
    GoalKind {
        /// The user's own daily water goal in ml.
        WaterDailyMl => "water_daily_ml",
    }
}

text_enum! {
    /// Kind of local sharing-policy operation (architecture 5.3).
    PolicyOpKind {
        /// Switch a category on (accepted only by the server).
        Enable => "enable",
        /// Switch a category off (always accepted, effective at once).
        Disable => "disable",
    }
}

text_enum! {
    /// State of a local policy operation.
    PolicyOpState {
        /// Not yet acknowledged by the server (or not paired).
        Pending => "pending",
        /// Acknowledged with a server sequence number.
        Accepted => "accepted",
        /// Refused as `stale_enable`; the device asks the user again.
        RejectedStale => "rejected_stale",
        /// Accepted during server recovery hold, sequenced later.
        PendingRecovery => "pending_recovery",
    }
}

text_enum! {
    /// State of a local deletion tombstone.
    TombstoneState {
        /// Recorded locally, not yet acknowledged.
        Pending => "pending",
        /// Acknowledged by the server.
        Acknowledged => "acknowledged",
        /// Accepted during server recovery hold, sequenced later.
        PendingRecovery => "pending_recovery",
    }
}

text_enum! {
    /// Where an imported summary row came from (architecture 5.4).
    ImportProvenance {
        /// A local `.hybackup` import. Takes precedence.
        BackupImport => "backup_import",
        /// "Restore from server".
        ServerRestore => "server_restore",
    }
}

text_enum! {
    /// Kinds of consent-log entry.
    ConsentKind {
        /// A notice version was shown.
        NoticeShown => "notice_shown",
        /// The user asked to switch a category on.
        EnableRequested => "enable_requested",
        /// The server accepted an enable.
        Enabled => "enabled",
        /// The user switched a category off.
        Disabled => "disabled",
        /// The user asked to delete shared data for a date range.
        DeletionRequested => "deletion_requested",
        /// An enable was refused as stale.
        StaleEnableRejected => "stale_enable_rejected",
        /// This PC paired with the office server (V1-5; the notice version is recorded).
        Paired => "paired",
        /// This PC unpaired, was revoked, or needs to pair again.
        Unpaired => "unpaired",
        /// An enable made on another of the user's PCs was applied here (owner O2).
        RemoteEnableApplied => "remote_enable_applied",
        /// The server saw an older receipt from this PC; every category was switched off.
        RegressionReset => "regression_reset",
    }
}

text_enum! {
    /// Where the pet sits (wireframe W3).
    PetEdge {
        /// Left screen edge.
        Left => "left",
        /// Right screen edge.
        Right => "right",
        /// Where the user dropped it.
        Free => "free",
    }
}

text_enum! {
    /// Wording tone (wireframe W3).
    PetTone {
        /// Gentle.
        Gentle => "gentle",
        /// Plain.
        Plain => "plain",
        /// Cheerful.
        Cheerful => "cheerful",
    }
}

/// Typed event payloads. The tag is the stored `type` and the content is the stored
/// `payload` JSON object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", deny_unknown_fields)]
pub enum EventPayload {
    /// The tracker entered a session state.
    #[serde(rename = "session.state")]
    SessionState {
        /// The new state.
        state: SessionState,
    },
    /// One activity was offered in the reminder bubble.
    #[serde(rename = "reminder.offered")]
    ReminderOffered {
        /// Identifier shared by the offer and its response.
        reminder_id: Uuid,
        /// The single activity offered.
        activity: Activity,
    },
    /// The outcome for the offered activity.
    #[serde(rename = "reminder.response")]
    ReminderResponse {
        /// The offer this answers.
        reminder_id: Uuid,
        /// The activity (always the offered one).
        activity: Activity,
        /// What happened.
        outcome: ReminderOutcome,
    },
    /// A water entry. A correction is a new `water.logged` that supersedes the old one.
    #[serde(rename = "water.logged")]
    WaterLogged {
        /// Amount in ml, 1 to 2000.
        amount_ml: u32,
    },
    /// One step of a movement routine.
    #[serde(rename = "movement.step")]
    MovementStep {
        /// The guided routine this step belongs to (shared with its `movement.session`).
        session_id: Uuid,
        /// Content id of the routine.
        routine_id: String,
        /// Content version of the routine.
        routine_version: u32,
        /// Seated or standing.
        variant: RoutineVariant,
        /// Zero-based step index.
        step_index: u32,
        /// Number of steps in the routine.
        steps_total: u32,
        /// Done or skipped.
        outcome: StepOutcome,
    },
    /// The result of a movement routine.
    #[serde(rename = "movement.session")]
    MovementSession {
        /// The guided routine (shared with its `movement.step` events).
        session_id: Uuid,
        /// Content id of the routine.
        routine_id: String,
        /// Content version of the routine.
        routine_version: u32,
        /// Seated or standing; absent when the day was skipped before choosing.
        variant: Option<RoutineVariant>,
        /// Overall result.
        outcome: RoutineOutcome,
        /// Steps done.
        steps_done: u32,
        /// Steps skipped.
        steps_skipped: u32,
        /// Steps in the routine.
        steps_total: u32,
    },
    /// A self-reported phone interruption. Editing the label supersedes it.
    #[serde(rename = "phone.interruption")]
    PhoneInterruption {
        /// Work, personal, unsure or unlabelled.
        label: PhoneLabel,
    },
    /// The answer to a deadline-day check-in. "Yes" marks the event's local date as
    /// deadline-heavy. Stays on this PC (architecture 6.2).
    #[serde(rename = "recovery.check_in")]
    RecoveryCheckIn {
        /// The long-day check-in or the pet menu.
        origin: CheckInOrigin,
        /// The answer.
        answer: CheckInReply,
    },
    /// The one pending recovery-day plan was saved or cancelled. Stays on this PC.
    #[serde(rename = "recovery.plan")]
    RecoveryPlanChanged {
        /// The planned recovery day.
        plan_date: NaiveDate,
        /// What changed.
        change: PlanChange,
    },
    /// The end-of-day reflection on a recovery day. Stays on this PC.
    #[serde(rename = "recovery.reflection")]
    RecoveryReflection {
        /// The answer ("Skip" records nothing).
        answer: ReflectionAnswer,
    },
    /// A meditation pause (V2-4; CAL-01): started from the daily invitation, text only.
    /// Stays on this PC and is never shared.
    #[serde(rename = "meditation.session")]
    MeditationSession {
        /// The pause's id.
        session_id: Uuid,
        /// The length chosen: 2, 5 or 10 minutes.
        minutes: u32,
        /// The version in use when it ended.
        version: MeditationVersion,
        /// Done or stopped.
        outcome: MeditationOutcome,
        /// Parts reached (1 to 5).
        parts_reached: u32,
    },
    /// A V2 posture observation (derived numbers only, never frames or landmarks).
    #[serde(rename = "posture.observation")]
    PostureObservation {
        /// What was observed.
        kind: PostureKind,
        /// Length of the interval or episode in seconds.
        duration_s: u32,
        /// Why (no-reading and suspect intervals only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<PostureReason>,
        /// What became of the cue (drift episodes only).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cue: Option<CueOutcome>,
    },
    /// A sharing category changed state on this device.
    #[serde(rename = "sharing.policy_changed")]
    SharingPolicyChanged {
        /// The category.
        category: Category,
        /// The new state.
        enabled: bool,
    },
    /// Undo or removal of a correctable entry. Supersedes the entry it removes.
    #[serde(rename = "entry.retracted")]
    EntryRetracted {},
}

impl EventPayload {
    /// The stored `type` value.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::SessionState { .. } => "session.state",
            Self::ReminderOffered { .. } => "reminder.offered",
            Self::ReminderResponse { .. } => "reminder.response",
            Self::WaterLogged { .. } => "water.logged",
            Self::MovementStep { .. } => "movement.step",
            Self::MovementSession { .. } => "movement.session",
            Self::RecoveryCheckIn { .. } => "recovery.check_in",
            Self::RecoveryPlanChanged { .. } => "recovery.plan",
            Self::RecoveryReflection { .. } => "recovery.reflection",
            Self::MeditationSession { .. } => "meditation.session",
            Self::PhoneInterruption { .. } => "phone.interruption",
            Self::PostureObservation { .. } => "posture.observation",
            Self::SharingPolicyChanged { .. } => "sharing.policy_changed",
            Self::EntryRetracted {} => "entry.retracted",
        }
    }

    /// Whether this is a user entry that can be corrected or retracted (WAT-01, DIS-01).
    #[must_use]
    pub const fn is_correctable(&self) -> bool {
        matches!(
            self,
            Self::WaterLogged { .. } | Self::PhoneInterruption { .. }
        )
    }

    /// Whether this payload may only come from the user.
    const fn requires_manual_source(&self) -> bool {
        matches!(
            self,
            Self::WaterLogged { .. }
                | Self::PhoneInterruption { .. }
                | Self::EntryRetracted {}
                | Self::MovementStep { .. }
                | Self::MovementSession { .. }
                | Self::RecoveryCheckIn { .. }
                | Self::RecoveryPlanChanged { .. }
                | Self::RecoveryReflection { .. }
                | Self::MeditationSession { .. }
        )
    }

    /// Split into the stored `type` and `payload` JSON text.
    ///
    /// # Errors
    /// Only if serialisation fails, which would be a defect.
    pub fn to_stored(&self) -> Result<(&'static str, String)> {
        let mut value = serde_json::to_value(self)?;
        let data = value
            .get_mut("data")
            .map(serde_json::Value::take)
            .ok_or_else(|| Error::invalid("payload has no data"))?;
        Ok((self.type_name(), serde_json::to_string(&data)?))
    }

    /// Rebuild from the stored `type` and `payload` JSON text.
    ///
    /// # Errors
    /// [`Error::Json`] if the type is unknown or the payload does not match it.
    pub fn from_stored(type_name: &str, payload: &str) -> Result<Self> {
        let data: serde_json::Value = serde_json::from_str(payload)?;
        Ok(serde_json::from_value(serde_json::json!({
            "type": type_name,
            "data": data,
        }))?)
    }

    /// Check the payload's own rules.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::WaterLogged { amount_ml } => {
                if !(1..=MAX_WATER_ENTRY_ML).contains(amount_ml) {
                    return Err(Error::invalid(format!(
                        "water amount {amount_ml} ml is outside 1 to {MAX_WATER_ENTRY_ML} ml"
                    )));
                }
            }
            Self::MovementStep {
                routine_id,
                step_index,
                steps_total,
                ..
            } => {
                check_routine_id(routine_id)?;
                if *steps_total == 0 || step_index >= steps_total {
                    return Err(Error::invalid("step index must be below steps_total"));
                }
            }
            Self::MovementSession {
                routine_id,
                variant,
                outcome,
                steps_done,
                steps_skipped,
                steps_total,
                ..
            } => {
                check_routine_id(routine_id)?;
                if *steps_total == 0 {
                    return Err(Error::invalid("a routine has at least one step"));
                }
                if steps_done + steps_skipped > *steps_total {
                    return Err(Error::invalid("done plus skipped steps exceed the routine"));
                }
                let consistent = match outcome {
                    RoutineOutcome::Done => *steps_done == *steps_total && variant.is_some(),
                    RoutineOutcome::Partial => {
                        *steps_done > 0 && *steps_done < *steps_total && variant.is_some()
                    }
                    RoutineOutcome::None => *steps_done == 0 && variant.is_some(),
                    RoutineOutcome::Skipped => *steps_done == 0 && *steps_skipped == 0,
                };
                if !consistent {
                    return Err(Error::invalid(format!(
                        "routine outcome {outcome} does not match {steps_done} of {steps_total} steps done"
                    )));
                }
            }
            Self::MeditationSession {
                minutes,
                outcome,
                parts_reached,
                ..
            } => {
                if !MEDITATION_MINUTES.contains(minutes) {
                    return Err(Error::invalid("a meditation pause is 2, 5 or 10 minutes"));
                }
                if !(1..=MEDITATION_PARTS).contains(parts_reached) {
                    return Err(Error::invalid("a meditation pause reaches 1 to 5 parts"));
                }
                if *outcome == MeditationOutcome::Done && *parts_reached != MEDITATION_PARTS {
                    return Err(Error::invalid("a done meditation pause reached every part"));
                }
            }
            Self::PostureObservation {
                kind,
                duration_s,
                reason,
                cue,
            } => {
                if *duration_s == 0 || *duration_s > 24 * 3600 {
                    return Err(Error::invalid("posture duration must be 1 s to 24 h"));
                }
                let consistent = match kind {
                    PostureKind::DriftEpisode => reason.is_none() && cue.is_some(),
                    PostureKind::NoReading => {
                        cue.is_none() && reason.is_some_and(PostureReason::is_no_reading)
                    }
                    PostureKind::BaselineSuspect => {
                        cue.is_none() && reason.is_some_and(|r| !r.is_no_reading())
                    }
                    PostureKind::Capture => cue.is_none() && reason.is_none(),
                };
                if !consistent {
                    return Err(Error::invalid(
                        "a drift episode has a cue outcome; no-reading and suspect intervals have their own reasons",
                    ));
                }
            }
            Self::SessionState { .. }
            | Self::ReminderOffered { .. }
            | Self::ReminderResponse { .. }
            | Self::RecoveryCheckIn { .. }
            | Self::RecoveryPlanChanged { .. }
            | Self::RecoveryReflection { .. }
            | Self::PhoneInterruption { .. }
            | Self::SharingPolicyChanged { .. }
            | Self::EntryRetracted {} => {}
        }
        Ok(())
    }
}

fn check_routine_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!("bad routine id {id:?}")))
    }
}

/// A stored event (architecture 6.1 `events`).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// UUIDv7.
    pub id: Uuid,
    /// Envelope and payload version.
    pub schema_version: u32,
    /// When it happened (UTC).
    pub occurred_at: DateTime<Utc>,
    /// When this PC recorded it (UTC). Later than `occurred_at` for backfilled entries.
    pub recorded_at: DateTime<Utc>,
    /// The IANA zone used for `local_date`.
    pub zone: UserZone,
    /// The local date, fixed when recorded; never rewritten by a later zone change.
    pub local_date: NaiveDate,
    /// Manual, inferred, system (imported is reserved).
    pub source: EventSource,
    /// V3 provenance reference; always `None` in V1 and V2.
    pub source_ref: Option<String>,
    /// Measurement status.
    pub status: MeasurementStatus,
    /// Confidence from 0 to 1 where meaningful.
    pub confidence: Option<f64>,
    /// The event this one corrects or retracts.
    pub supersedes: Option<Uuid>,
    /// The PC that recorded the event; kept through export, import and restore.
    pub origin_device: Uuid,
    /// The typed payload.
    pub payload: EventPayload,
}

/// An event to record. The store fills in the id, times, zone, local date and origin.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    /// The typed payload.
    pub payload: EventPayload,
    /// Source.
    pub source: EventSource,
    /// Measurement status.
    pub status: MeasurementStatus,
    /// Confidence from 0 to 1.
    pub confidence: Option<f64>,
    /// When it happened; defaults to now (or to the superseded event's time).
    pub occurred_at: Option<DateTime<Utc>>,
    /// The event this corrects or retracts.
    pub supersedes: Option<Uuid>,
    /// V3 provenance; must be `None` in V1 and V2.
    pub source_ref: Option<String>,
}

impl NewEvent {
    /// A user entry: source manual, status self-reported.
    #[must_use]
    pub fn manual(payload: EventPayload) -> Self {
        Self {
            payload,
            source: EventSource::Manual,
            status: MeasurementStatus::SelfReported,
            confidence: None,
            occurred_at: None,
            supersedes: None,
            source_ref: None,
        }
    }

    /// An app-derived event.
    #[must_use]
    pub fn inferred(payload: EventPayload, status: MeasurementStatus) -> Self {
        Self {
            source: EventSource::Inferred,
            status,
            ..Self::manual(payload)
        }
    }

    /// A Windows-reported event, status measured.
    #[must_use]
    pub fn system(payload: EventPayload) -> Self {
        Self {
            source: EventSource::System,
            status: MeasurementStatus::Measured,
            ..Self::manual(payload)
        }
    }

    /// Set when the event happened (for backfilled entries).
    #[must_use]
    pub fn at(mut self, occurred_at: DateTime<Utc>) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }

    /// Mark this event as correcting or retracting `id`.
    #[must_use]
    pub fn superseding(mut self, id: Uuid) -> Self {
        self.supersedes = Some(id);
        self
    }

    /// Set the confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Check envelope rules that do not need the store.
    ///
    /// # Errors
    /// [`Error::Invalid`] naming the broken rule.
    pub fn validate(&self) -> Result<()> {
        self.payload.validate()?;
        if self.source == EventSource::Imported || self.source_ref.is_some() {
            return Err(Error::invalid(
                "imported events are reserved for V3 and refused in V1 and V2",
            ));
        }
        if let Some(c) = self.confidence
            && !(0.0..=1.0).contains(&c)
        {
            return Err(Error::invalid(format!("confidence {c} is outside 0 to 1")));
        }
        let manual = self.source == EventSource::Manual;
        let self_reported = self.status == MeasurementStatus::SelfReported;
        if manual != self_reported {
            return Err(Error::invalid(
                "manual events are self-reported, and only manual events are",
            ));
        }
        if self.payload.requires_manual_source() && !manual {
            return Err(Error::invalid(format!(
                "{} must come from the user",
                self.payload.type_name()
            )));
        }
        if matches!(self.payload, EventPayload::SessionState { .. }) && manual {
            return Err(Error::invalid("session states come from the tracker"));
        }
        let retraction = matches!(self.payload, EventPayload::EntryRetracted {});
        if retraction && self.supersedes.is_none() {
            return Err(Error::invalid(
                "a retraction must name the entry it removes",
            ));
        }
        if self.supersedes.is_some() && !retraction && !self.payload.is_correctable() {
            return Err(Error::invalid(format!(
                "{} cannot supersede another event",
                self.payload.type_name()
            )));
        }
        Ok(())
    }
}

/// An inclusive range of local dates, used by deletion tombstones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    /// First date (inclusive).
    pub from: NaiveDate,
    /// Last date (inclusive).
    pub to: NaiveDate,
}

impl DateRange {
    /// A range from `from` to `to`, both included.
    ///
    /// # Errors
    /// [`Error::Invalid`] if `to` is before `from`.
    pub fn new(from: NaiveDate, to: NaiveDate) -> Result<Self> {
        if to < from {
            return Err(Error::invalid("date range ends before it starts"));
        }
        Ok(Self { from, to })
    }

    /// "All dates" (wireframe M2).
    #[must_use]
    pub fn all() -> Self {
        Self {
            from: NaiveDate::from_ymd_opt(1, 1, 1).unwrap_or(NaiveDate::MIN),
            to: NaiveDate::from_ymd_opt(9999, 12, 31).unwrap_or(NaiveDate::MAX),
        }
    }

    /// One date.
    #[must_use]
    pub fn day(date: NaiveDate) -> Self {
        Self {
            from: date,
            to: date,
        }
    }

    /// Whether `date` is inside the range.
    #[must_use]
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.from <= date && date <= self.to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn water(ml: u32) -> EventPayload {
        EventPayload::WaterLogged { amount_ml: ml }
    }

    fn session(outcome: RoutineOutcome, done: u32, skipped: u32, total: u32) -> EventPayload {
        EventPayload::MovementSession {
            session_id: Uuid::nil(),
            routine_id: "mon-neck-shoulders".into(),
            routine_version: 1,
            variant: Some(RoutineVariant::Seated),
            outcome,
            steps_done: done,
            steps_skipped: skipped,
            steps_total: total,
        }
    }

    fn all_payload_samples() -> Vec<EventPayload> {
        let rid = Uuid::nil();
        vec![
            EventPayload::SessionState {
                state: SessionState::Quiet,
            },
            EventPayload::ReminderOffered {
                reminder_id: rid,
                activity: Activity::Water,
            },
            EventPayload::ReminderResponse {
                reminder_id: rid,
                activity: Activity::EyeRest,
                outcome: ReminderOutcome::NoResponse,
            },
            water(250),
            EventPayload::MovementStep {
                session_id: rid,
                routine_id: "tue-back".into(),
                routine_version: 2,
                variant: RoutineVariant::Standing,
                step_index: 0,
                steps_total: 3,
                outcome: StepOutcome::Skipped,
            },
            session(RoutineOutcome::Partial, 2, 1, 3),
            EventPayload::RecoveryCheckIn {
                origin: CheckInOrigin::LongDay,
                answer: CheckInReply::Yes,
            },
            EventPayload::RecoveryPlanChanged {
                plan_date: NaiveDate::from_ymd_opt(2026, 10, 23).unwrap(),
                change: PlanChange::Saved,
            },
            EventPayload::RecoveryReflection {
                answer: ReflectionAnswer::Somewhat,
            },
            EventPayload::PhoneInterruption {
                label: PhoneLabel::Work,
            },
            EventPayload::MeditationSession {
                session_id: Uuid::nil(),
                minutes: 5,
                version: MeditationVersion::EyesOpen,
                outcome: MeditationOutcome::Done,
                parts_reached: 5,
            },
            EventPayload::PostureObservation {
                kind: PostureKind::DriftEpisode,
                duration_s: 75,
                reason: None,
                cue: Some(CueOutcome::Shown),
            },
            EventPayload::SharingPolicyChanged {
                category: Category::Hydration,
                enabled: false,
            },
            EventPayload::EntryRetracted {},
        ]
    }

    #[test]
    fn text_enums_round_trip_through_text_and_serde() {
        fn check<T>(all: &[T])
        where
            T: Copy
                + fmt::Debug
                + PartialEq
                + FromStr<Err = Error>
                + Serialize
                + for<'a> Deserialize<'a>
                + fmt::Display,
        {
            for v in all {
                let text = v.to_string();
                assert_eq!(text.parse::<T>().unwrap(), *v);
                let json = serde_json::to_string(v).unwrap();
                assert_eq!(json, format!("\"{text}\""));
                assert_eq!(serde_json::from_str::<T>(&json).unwrap(), *v);
            }
            assert!("not-a-value".parse::<T>().is_err());
        }
        check(EventSource::ALL);
        check(MeasurementStatus::ALL);
        check(Category::ALL);
        check(Activity::ALL);
        check(ReminderOutcome::ALL);
        check(RoutineVariant::ALL);
        check(RoutineOutcome::ALL);
        check(StepOutcome::ALL);
        check(CheckInOrigin::ALL);
        check(CheckInReply::ALL);
        check(PlanChange::ALL);
        check(ReflectionAnswer::ALL);
        check(MeditationVersion::ALL);
        check(MeditationOutcome::ALL);
        check(PhoneLabel::ALL);
        check(SessionState::ALL);
        check(PostureKind::ALL);
        check(PostureReason::ALL);
        check(CueOutcome::ALL);
        check(GoalKind::ALL);
        check(PolicyOpKind::ALL);
        check(PolicyOpState::ALL);
        check(TombstoneState::ALL);
        check(ImportProvenance::ALL);
        check(ConsentKind::ALL);
        check(PetEdge::ALL);
        check(PetTone::ALL);
    }

    #[test]
    fn reminder_outcomes_cover_the_approved_statuses() {
        // Wireframe acceptance 2.4: done, snoozed, skipped, dismissed, offered with no response.
        for name in ["done", "snoozed", "skipped", "dismissed", "no_response"] {
            assert!(name.parse::<ReminderOutcome>().is_ok(), "{name}");
        }
        for name in ["done", "partial", "none", "skipped"] {
            assert!(name.parse::<RoutineOutcome>().is_ok(), "{name}");
        }
    }

    #[test]
    fn only_done_and_partial_reset_the_movement_timer() {
        assert!(RoutineOutcome::Done.resets_movement_timer());
        assert!(RoutineOutcome::Partial.resets_movement_timer());
        assert!(!RoutineOutcome::None.resets_movement_timer());
        assert!(!RoutineOutcome::Skipped.resets_movement_timer());
    }

    #[test]
    fn payloads_round_trip_through_stored_form() {
        let samples = all_payload_samples();
        let mut names = std::collections::HashSet::new();
        for p in samples {
            p.validate().unwrap();
            let (type_name, json) = p.to_stored().unwrap();
            assert_eq!(type_name, p.type_name());
            assert!(names.insert(type_name), "duplicate type name {type_name}");
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(value.is_object(), "{type_name} payload must be an object");
            assert_eq!(EventPayload::from_stored(type_name, &json).unwrap(), p);
        }
    }

    #[test]
    fn stored_payloads_reject_unknown_types_and_fields() {
        assert!(EventPayload::from_stored("water.logged", r#"{"amount_ml":250}"#).is_ok());
        assert!(EventPayload::from_stored("water.poured", r#"{"amount_ml":250}"#).is_err());
        assert!(EventPayload::from_stored("water.logged", r#"{"amount_ml":"lots"}"#).is_err());
        assert!(
            EventPayload::from_stored("water.logged", r#"{"amount_ml":250,"note":"x"}"#).is_err()
        );
        assert!(EventPayload::from_stored("water.logged", "not json").is_err());
    }

    #[test]
    fn water_amount_limits() {
        assert!(water(0).validate().is_err());
        assert!(water(1).validate().is_ok());
        assert!(water(MAX_WATER_ENTRY_ML).validate().is_ok());
        assert!(water(MAX_WATER_ENTRY_ML + 1).validate().is_err());
    }

    #[test]
    fn routine_outcome_must_match_step_counts() {
        assert!(session(RoutineOutcome::Done, 3, 0, 3).validate().is_ok());
        assert!(session(RoutineOutcome::Done, 2, 1, 3).validate().is_err());
        assert!(session(RoutineOutcome::Partial, 2, 1, 3).validate().is_ok());
        assert!(
            session(RoutineOutcome::Partial, 3, 0, 3)
                .validate()
                .is_err()
        );
        assert!(
            session(RoutineOutcome::Partial, 0, 3, 3)
                .validate()
                .is_err()
        );
        assert!(session(RoutineOutcome::None, 0, 3, 3).validate().is_ok());
        assert!(session(RoutineOutcome::None, 0, 0, 3).validate().is_ok()); // immediate Stop
        assert!(session(RoutineOutcome::None, 1, 0, 3).validate().is_err());
        assert!(session(RoutineOutcome::Done, 3, 1, 3).validate().is_err());
        assert!(session(RoutineOutcome::Done, 0, 0, 0).validate().is_err());
        let skipped_day = EventPayload::MovementSession {
            session_id: Uuid::nil(),
            routine_id: "wed-hips".into(),
            routine_version: 1,
            variant: None,
            outcome: RoutineOutcome::Skipped,
            steps_done: 0,
            steps_skipped: 0,
            steps_total: 3,
        };
        assert!(skipped_day.validate().is_ok());
        let done_without_variant = EventPayload::MovementSession {
            session_id: Uuid::nil(),
            routine_id: "wed-hips".into(),
            routine_version: 1,
            variant: None,
            outcome: RoutineOutcome::Done,
            steps_done: 3,
            steps_skipped: 0,
            steps_total: 3,
        };
        assert!(done_without_variant.validate().is_err());
    }

    #[test]
    fn step_and_routine_ids_are_checked() {
        let step = |idx, total, id: &str| EventPayload::MovementStep {
            session_id: Uuid::nil(),
            routine_id: id.into(),
            routine_version: 1,
            variant: RoutineVariant::Seated,
            step_index: idx,
            steps_total: total,
            outcome: StepOutcome::Done,
        };
        assert!(step(2, 3, "a-1").validate().is_ok());
        assert!(step(3, 3, "a-1").validate().is_err());
        assert!(step(0, 0, "a-1").validate().is_err());
        assert!(step(0, 3, "").validate().is_err());
        assert!(step(0, 3, "Has Space").validate().is_err());
        assert!(step(0, 3, &"x".repeat(65)).validate().is_err());
    }

    #[test]
    fn posture_duration_limits() {
        let p = |d| EventPayload::PostureObservation {
            kind: PostureKind::NoReading,
            duration_s: d,
            reason: Some(PostureReason::Absent),
            cue: None,
        };
        assert!(p(0).validate().is_err());
        assert!(p(1).validate().is_ok());
        assert!(p(24 * 3600 + 1).validate().is_err());
    }

    #[test]
    fn meditation_session_rules() {
        let m = |minutes, outcome, parts_reached| EventPayload::MeditationSession {
            session_id: Uuid::nil(),
            minutes,
            version: MeditationVersion::Breath,
            outcome,
            parts_reached,
        };
        for minutes in MEDITATION_MINUTES {
            assert!(m(minutes, MeditationOutcome::Done, 5).validate().is_ok());
        }
        assert!(m(3, MeditationOutcome::Done, 5).validate().is_err());
        assert!(m(5, MeditationOutcome::Stopped, 1).validate().is_ok());
        assert!(m(5, MeditationOutcome::Stopped, 0).validate().is_err());
        assert!(m(5, MeditationOutcome::Stopped, 6).validate().is_err());
        assert!(m(5, MeditationOutcome::Done, 4).validate().is_err());
        // Only the user starts one.
        let mut e = NewEvent::manual(m(5, MeditationOutcome::Done, 5));
        assert!(e.validate().is_ok());
        e.source = EventSource::Inferred;
        assert!(e.validate().is_err());
    }

    #[test]
    fn posture_reasons_and_cues_belong_to_their_kinds() {
        let p = |kind, reason, cue| EventPayload::PostureObservation {
            kind,
            duration_s: 60,
            reason,
            cue,
        };
        use PostureKind::{BaselineSuspect, DriftEpisode, NoReading};
        assert!(
            p(DriftEpisode, None, Some(CueOutcome::Focus))
                .validate()
                .is_ok()
        );
        assert!(p(DriftEpisode, None, None).validate().is_err());
        assert!(
            p(
                DriftEpisode,
                Some(PostureReason::Absent),
                Some(CueOutcome::Shown)
            )
            .validate()
            .is_err()
        );
        assert!(
            p(NoReading, Some(PostureReason::Stalled), None)
                .validate()
                .is_ok()
        );
        assert!(
            p(NoReading, Some(PostureReason::CameraMoved), None)
                .validate()
                .is_err()
        );
        assert!(p(NoReading, None, None).validate().is_err());
        assert!(
            p(BaselineSuspect, Some(PostureReason::ViewpointChanged), None)
                .validate()
                .is_ok()
        );
        assert!(
            p(BaselineSuspect, Some(PostureReason::Absent), None)
                .validate()
                .is_err()
        );
        assert!(
            p(
                BaselineSuspect,
                Some(PostureReason::AppRestart),
                Some(CueOutcome::Shown)
            )
            .validate()
            .is_err()
        );
        // V2-3: camera-on time has neither; time without a calibration is a suspect kind.
        use PostureKind::Capture;
        assert!(p(Capture, None, None).validate().is_ok());
        assert!(
            p(Capture, Some(PostureReason::NoCalibration), None)
                .validate()
                .is_err()
        );
        assert!(
            p(Capture, None, Some(CueOutcome::Shown))
                .validate()
                .is_err()
        );
        assert!(
            p(BaselineSuspect, Some(PostureReason::NoCalibration), None)
                .validate()
                .is_ok()
        );
        assert!(
            p(NoReading, Some(PostureReason::NoCalibration), None)
                .validate()
                .is_err()
        );
        // Stored without the optional fields, an episode is refused (V2-2 always writes
        // them; nothing earlier wrote posture events).
        let old: EventPayload = serde_json::from_str(
            r#"{"type":"posture.observation","data":{"kind":"drift_episode","duration_s":75}}"#,
        )
        .unwrap();
        assert!(old.validate().is_err());
    }

    #[test]
    fn envelope_rules() {
        assert!(NewEvent::manual(water(250)).validate().is_ok());
        // Imported is reserved for V3.
        let mut e = NewEvent::manual(water(250));
        e.source = EventSource::Imported;
        assert!(e.validate().is_err());
        let mut e = NewEvent::manual(water(250));
        e.source_ref = Some("watch:123".into());
        assert!(e.validate().is_err());
        // Manual and self-reported go together.
        let mut e = NewEvent::manual(water(250));
        e.status = MeasurementStatus::Measured;
        assert!(e.validate().is_err());
        let tracker = NewEvent::inferred(
            EventPayload::SessionState {
                state: SessionState::Quiet,
            },
            MeasurementStatus::SelfReported,
        );
        assert!(tracker.validate().is_err());
        // User entries cannot be inferred; session states cannot be manual.
        assert!(
            NewEvent::inferred(water(250), MeasurementStatus::Estimated)
                .validate()
                .is_err()
        );
        let manual_state = NewEvent::manual(EventPayload::SessionState {
            state: SessionState::Active,
        });
        assert!(manual_state.validate().is_err());
        assert!(
            NewEvent::system(EventPayload::SessionState {
                state: SessionState::ShortLock
            })
            .validate()
            .is_ok()
        );
        // Confidence range.
        let quiet = || {
            NewEvent::inferred(
                EventPayload::SessionState {
                    state: SessionState::Quiet,
                },
                MeasurementStatus::Estimated,
            )
        };
        assert!(quiet().with_confidence(0.6).validate().is_ok());
        assert!(quiet().with_confidence(1.5).validate().is_err());
        assert!(quiet().with_confidence(f64::NAN).validate().is_err());
        // Supersede rules.
        assert!(
            NewEvent::manual(EventPayload::EntryRetracted {})
                .validate()
                .is_err()
        );
        assert!(
            NewEvent::manual(EventPayload::EntryRetracted {})
                .superseding(Uuid::nil())
                .validate()
                .is_ok()
        );
        assert!(quiet().superseding(Uuid::nil()).validate().is_err());
        assert!(
            NewEvent::manual(water(300))
                .superseding(Uuid::nil())
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn date_ranges() {
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let r = DateRange::new(d(10, 12), d(10, 16)).unwrap();
        assert!(r.contains(d(10, 12)) && r.contains(d(10, 16)) && !r.contains(d(10, 17)));
        assert!(DateRange::new(d(10, 16), d(10, 12)).is_err());
        assert!(DateRange::all().contains(d(1, 1)));
        assert!(DateRange::day(d(10, 21)).contains(d(10, 21)));
        assert!(!DateRange::day(d(10, 21)).contains(d(10, 22)));
    }
}
