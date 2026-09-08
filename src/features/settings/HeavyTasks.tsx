/**
 * T546 — how many heavy tasks may run at once.
 *
 * **Why this needed a screen at all.** `Settings.concurrent_heavy_tasks` reaches the task
 * engine and changes its limit without a restart — the backend half of this was already done
 * (`agent/backend`, T546) — but nothing on any screen showed the number or let a person change
 * it. It lived only in the database and in test fixtures: a setting nobody can reach is not a
 * setting, it is a default with an extra column.
 *
 * **What "heavy" means here.** Conversions, uploads, quality builds and deployments — the
 * tasks that saturate a disk or a network link rather than finishing in a second. Running too
 * many of them at once is what turns a healthy machine into one where every one of them
 * crawls; this is the knob that trades total throughput for per-task speed.
 *
 * **The ceiling is not repeated here on purpose.** `MAX_HEAVY_TASKS` lives once, in
 * `store/settings.rs`, and `Settings::clamped()` enforces it on every `settings_set`. Printing
 * `max={8}` in this file would be a second copy of that number, free to drift from the first
 * the day either one changes — so there is no `max` here, only the universal "not below one"
 * that `LimitDialog.tsx` already uses for a number nobody could sensibly type negative. What
 * this screen shows after saving is whatever the core actually kept, not whatever was typed.
 */

import { useSettings } from "../../app/settings";
import { useT } from "../../shared/i18n";

export function HeavyTasks() {
  const t = useT();
  const words = t.ui.appearance;
  const { settings, update } = useSettings();
  const value = settings?.concurrent_heavy_tasks ?? 1;

  return (
    <fieldset>
      <legend>{words.heavyTasks}</legend>
      <p className="appearance__means">{words.heavyTasksMeans}</p>
      <label>
        <input
          type="number"
          min={1}
          value={value}
          disabled={settings === null}
          onChange={(e) =>
            update({ concurrent_heavy_tasks: Math.max(1, Number(e.target.value)) })
          }
          data-testid="heavy-tasks-input"
        />
      </label>
    </fieldset>
  );
}
