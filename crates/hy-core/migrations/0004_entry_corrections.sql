-- HealthierYou local store, schema v4: water and phone entry corrections (slice V1-3
-- follow-up, wireframes B3 and B8).
-- Forward-only and additive ("expand"): a v3 build can still open a v4 database.
-- Once released, this file must never change: a checksum test guards it.
-- Only a trigger on the existing event log; no table changes.

-- Undo, Remove, Edit and a new phone label are new events that supersede the entry
-- (architecture 6.1), so totals recalculate. A correction keeps the type of the entry it
-- corrects; anything else that supersedes an entry is a retraction ('entry.retracted').
-- A retraction is final: it is never itself corrected or retracted.
CREATE TRIGGER events_correction_keeps_type BEFORE INSERT ON events
WHEN NEW.supersedes IS NOT NULL AND EXISTS (
    SELECT 1 FROM events t
    WHERE t.id = NEW.supersedes
      AND (t.type = 'entry.retracted'
           OR (NEW.type <> t.type AND NEW.type <> 'entry.retracted'))
)
BEGIN
    SELECT RAISE(ABORT, 'a correction keeps the entry''s type, and a retraction is final');
END;
