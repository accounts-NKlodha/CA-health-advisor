// Settings > Water and workweek (wireframe W5): the user's own water goal, the serving
// buttons, the pacing switch (the same setting as Reminders > Water pacing), then the
// workdays, work hours, weekly review time and time zone. The fields show the saved
// values whenever the tab opens; edits are kept only when saved.
import { useEffect, useState } from 'preact/hooks';
import { call } from '../shared/ipc';
import type { SettingsPage, WeekdayName } from '../shared/types';
import type { Save } from './main';

const DAYS: [WeekdayName, string][] = [
  ['mon', 'Monday'],
  ['tue', 'Tuesday'],
  ['wed', 'Wednesday'],
  ['thu', 'Thursday'],
  ['fri', 'Friday'],
  ['sat', 'Saturday'],
  ['sun', 'Sunday'],
];

function Water({ page, save }: { page: SettingsPage; save: Save }) {
  const [goal, setGoal] = useState(page.water_goal_ml === null ? '' : String(page.water_goal_ml));
  const [add, setAdd] = useState('');
  useEffect(() => {
    setGoal(page.water_goal_ml === null ? '' : String(page.water_goal_ml));
  }, [page.water_goal_ml]);

  const servings = (list: number[], done: string) =>
    save(call('save_water_settings', { goalMl: null, servings: list }), done);
  const goalNumber = Number(goal);
  const goalValid =
    goal !== '' && Number.isInteger(goalNumber) && goalNumber >= page.goal_min_ml && goalNumber <= page.goal_max_ml;
  const addNumber = Number(add);
  const addValid =
    add !== '' &&
    Number.isInteger(addNumber) &&
    addNumber >= page.serving_min_ml &&
    addNumber <= page.serving_max_ml &&
    addNumber % 10 === 0 &&
    !page.servings.includes(addNumber);
  const full = page.servings.length >= page.max_servings;

  return (
    <section aria-labelledby="water-h">
      <h3 id="water-h">Water</h3>
      <form
        class="inline"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          if (!goalValid) return;
          save(
            call('save_water_settings', { goalMl: goalNumber, servings: page.servings }),
            `Daily goal ${goalNumber} ml, from today.`,
          );
        }}
      >
        <label class="inline" for="goal">
          Daily water goal (ml)
        </label>
        <input
          id="goal"
          type="number"
          min={page.goal_min_ml}
          max={page.goal_max_ml}
          step={50}
          value={goal}
          aria-describedby="goal-note"
          onInput={(e) => setGoal(e.currentTarget.value)}
        />
        <button type="submit" class="btn" disabled={!goalValid || goalNumber === page.water_goal_ml}>
          Save goal
        </button>
      </form>
      <p class="note" id="goal-note">
        Your own choice, {page.goal_min_ml} to {page.goal_max_ml} ml. HealthierYou never suggests
        or raises it. Goal changes apply from today; past weeks are judged against the goal you had
        then.
      </p>

      <fieldset class="field">
        <legend>Serving buttons</legend>
        <div class="actions">
          {page.servings.map((ml) => (
            <span class="serving" key={ml}>
              {ml} ml
              <button
                type="button"
                class="btn link"
                aria-label={`Remove the ${ml} ml button`}
                disabled={page.servings.length <= 1}
                onClick={() =>
                  servings(
                    page.servings.filter((x) => x !== ml),
                    `${ml} ml button removed.`,
                  )
                }
              >
                Remove
              </button>
            </span>
          ))}
        </div>
        <form
          class="inline"
          onSubmit={(e) => {
            e.preventDefault();
            if (!addValid || full) return;
            servings(
              [...page.servings, addNumber].sort((a, b) => a - b),
              `${addNumber} ml button added.`,
            );
            setAdd('');
          }}
        >
          <label for="serving-add">New button (ml)</label>
          <input
            id="serving-add"
            type="number"
            min={page.serving_min_ml}
            max={page.serving_max_ml}
            step={10}
            value={add}
            disabled={full}
            onInput={(e) => setAdd(e.currentTarget.value)}
          />
          <button type="submit" class="btn" disabled={!addValid || full}>
            Add
          </button>
        </form>
        <p class="note">
          Up to {page.max_servings} buttons, each {page.serving_min_ml} to {page.serving_max_ml} ml
          in steps of 10. They appear in the pet menu and the water panel.
        </p>
      </fieldset>

      <label class="check block">
        <input
          type="checkbox"
          role="switch"
          checked={page.coach.categories.water}
          onChange={(e) => {
            const on = e.currentTarget.checked;
            save(
              call('save_coach_settings', {
                config: { ...page.coach, categories: { ...page.coach.categories, water: on } },
              }),
              on ? 'Water pacing on.' : 'Water pacing off.',
            );
          }}
        />
        <span>Water pacing reminders within work hours (also under Reminders)</span>
      </label>
    </section>
  );
}

function Workweek({ page, save }: { page: SettingsPage; save: Save }) {
  const [days, setDays] = useState<WeekdayName[]>(page.schedule.workdays);
  const [start, setStart] = useState(page.schedule.start);
  const [end, setEnd] = useState(page.schedule.end);
  const [zone, setZone] = useState(page.zone);
  const [reviewDay, setReviewDay] = useState<WeekdayName>(page.coach.review.day);
  const [reviewAt, setReviewAt] = useState(page.coach.review.at);
  const reset = () => {
    setDays(page.schedule.workdays);
    setStart(page.schedule.start);
    setEnd(page.schedule.end);
    setZone(page.zone);
    setReviewDay(page.coach.review.day);
    setReviewAt(page.coach.review.at);
  };
  useEffect(reset, [page]);

  const reviewChanged = reviewDay !== page.coach.review.day || reviewAt !== page.coach.review.at;
  const scheduleChanged =
    start !== page.schedule.start ||
    end !== page.schedule.end ||
    zone !== page.zone ||
    days.join() !== page.schedule.workdays.join();
  const valid = start !== '' && end !== '' && start !== end && reviewAt !== '';

  const submit = () => {
    if (!valid) return;
    const ordered = DAYS.map(([d]) => d).filter((d) => days.includes(d));
    const work = scheduleChanged
      ? call('save_workweek', { schedule: { workdays: ordered, start, end }, zone })
      : Promise.resolve(page);
    save(
      work.then((p) =>
        reviewChanged
          ? call('save_coach_settings', { config: { ...p.coach, review: { day: reviewDay, at: reviewAt } } })
          : p,
      ),
      'Workweek saved.',
    );
  };

  return (
    <section aria-labelledby="week-h">
      <h3 id="week-h">Workweek</h3>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <fieldset class="field">
          <legend>Workdays</legend>
          <div class="actions">
            {DAYS.map(([d, label]) => (
              <label class="check" key={d}>
                <input
                  type="checkbox"
                  checked={days.includes(d)}
                  onChange={(e) =>
                    setDays(e.currentTarget.checked ? [...days, d] : days.filter((x) => x !== d))
                  }
                />
                {label}
                {d === 'sun' ? ' (optional)' : ''}
              </label>
            ))}
          </div>
        </fieldset>
        <div class="inline">
          <label for="hours-start">Work hours</label>
          <input id="hours-start" type="time" value={start} onInput={(e) => setStart(e.currentTarget.value)} />
          <label for="hours-end">to</label>
          <input id="hours-end" type="time" value={end} onInput={(e) => setEnd(e.currentTarget.value)} />
        </div>
        <p class="note">Reminders are offered only inside these hours on workdays. An end before the start runs past midnight.</p>
        <div class="inline">
          <label for="review-day">Weekly review</label>
          <select
            id="review-day"
            value={reviewDay}
            onChange={(e) => setReviewDay(e.currentTarget.value as WeekdayName)}
          >
            {DAYS.map(([d, label]) => (
              <option value={d} key={d}>
                {label}
              </option>
            ))}
          </select>
          <label for="review-at">at</label>
          <input id="review-at" type="time" value={reviewAt} onInput={(e) => setReviewAt(e.currentTarget.value)} />
        </div>
        <p class="note">The pet offers the week's review then, once a week. Open, Later or Skip.</p>
        <label class="field">
          <span>Time zone</span>
          <select value={zone} onChange={(e) => setZone(e.currentTarget.value)}>
            {page.zones.map((z) => (
              <option value={z} key={z}>
                {z}
              </option>
            ))}
          </select>
        </label>
        <div class="actions">
          <button type="submit" class="btn primary" disabled={!valid || (!scheduleChanged && !reviewChanged)}>
            Save workweek
          </button>
          <button type="button" class="btn" disabled={!scheduleChanged && !reviewChanged} onClick={reset}>
            Discard changes
          </button>
        </div>
        {start !== '' && start === end ? (
          <p class="note" role="alert">
            Work hours need a different start and end.
          </p>
        ) : null}
      </form>
    </section>
  );
}

export function WaterTab({ page, save }: { page: SettingsPage; save: Save }) {
  return (
    <main class="pane">
      <h2>Water & workweek</h2>
      <Water page={page} save={save} />
      <Workweek page={page} save={save} />
    </main>
  );
}
