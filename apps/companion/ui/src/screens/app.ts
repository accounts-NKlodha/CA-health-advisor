// The text of Settings > App & updates (slice V1-6), kept apart from the component so it
// can be tested with node --test.
import type { AppPage, UpdateCheck, UpdateOutcome } from '../shared/types';

/** What installing the checked file will do. */
export function checkSummary(c: UpdateCheck): string {
  const what =
    c.plan === 'upgrade'
      ? `Update from version ${c.current} to ${c.version}`
      : `Go back from version ${c.current} to ${c.version}, the version this PC was updated from`;
  return `${what} (${c.size_mb} MB). The signature checked out.`;
}

/** The warning for a file Windows marked as downloaded. */
export function markNote(c: UpdateCheck): string | null {
  if (!c.marked_from_internet) return null;
  return 'Windows marked this file as downloaded, so it may show a warning such as "Windows protected your PC" or "Unknown publisher" before the installer runs. HealthierYou does not switch that check off. If Windows blocks the installer, nothing is changed: tell the owner, who decides about signing.';
}

/** The last update, in one line. */
export function outcomeLine(o: UpdateOutcome): string {
  switch (o.kind) {
    case 'updated':
      return `${o.at}: updated from ${o.from} to ${o.to}.`;
    case 'rolled_back':
      return `${o.at}: the update to ${o.to} was undone. ${o.detail}`;
    case 'blocked':
      return `${o.at}: the update to ${o.to} did not run. ${o.detail}`;
    case 'failed':
      return `${o.at}: the update to ${o.to} failed. ${o.detail}`;
  }
}

/** Start at sign-in, in one line. */
export function autostartLine(a: NonNullable<AppPage['autostart']>): string {
  if (a.enabled && a.switched_off_in_windows) {
    return 'Switched off in Windows (Task Manager > Startup apps). Switch it on there to start HealthierYou at sign-in.';
  }
  return a.enabled
    ? 'HealthierYou starts when you sign in to Windows.'
    : 'HealthierYou does not start by itself. Open it from the Start menu.';
}

/** What going back is possible to, if anything. */
export function rollbackLine(p: AppPage): string {
  return p.previous_version
    ? `To go back, install version ${p.previous_version} (the version this PC was updated from) from its file the same way.`
    : 'Going back is possible only to the version this PC was updated from; there is none yet.';
}
