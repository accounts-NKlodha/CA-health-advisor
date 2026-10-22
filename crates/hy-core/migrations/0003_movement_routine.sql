-- HealthierYou local store, schema v3: the guided movement routine (slice V1-3).
-- Forward-only and additive ("expand"): a v2 build can still open a v3 database.
-- Once released, this file must never change: a checksum test guards it.
-- Only indexes and triggers on the existing event log; no table changes.

-- A guided routine has one result, and each of its steps is answered once. The shared
-- `session_id` ties a routine's `movement.step` events to its `movement.session`. A later
-- correction (an event that supersedes one of these) is not a second answer.
CREATE UNIQUE INDEX events_one_result_per_routine
    ON events (json_extract(payload, '$.session_id'))
    WHERE type = 'movement.session' AND supersedes IS NULL;

CREATE UNIQUE INDEX events_one_answer_per_routine_step
    ON events (json_extract(payload, '$.session_id'), json_extract(payload, '$.step_index'))
    WHERE type = 'movement.step' AND supersedes IS NULL;

-- Steps are answered before the routine's result, never after it.
CREATE TRIGGER events_step_before_routine_result BEFORE INSERT ON events
WHEN NEW.type = 'movement.step' AND NEW.supersedes IS NULL AND EXISTS (
    SELECT 1 FROM events r
    WHERE r.type = 'movement.session'
      AND json_extract(r.payload, '$.session_id') = json_extract(NEW.payload, '$.session_id')
)
BEGIN
    SELECT RAISE(ABORT, 'a routine step cannot follow the routine result');
END;
