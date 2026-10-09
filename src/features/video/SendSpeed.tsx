/**
 * T701 — how fast the «Video» screen sends rungs to the server: «No limit» or N Mbit/s.
 *
 * One choice for the whole screen, not one per video: what matters is how much of the channel
 * is left to friends watching, and two rungs sent at once share the cap. It reaches a rung
 * already on its way, and it is kept (`Settings.send_limit_bps`, bytes per second, as every
 * speed in the core). A pause stops the preparation outright; this only slows the sending.
 */

import { useSettings } from "../../app/settings";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";

/** The choices, in Mbit/s. */
export const SEND_MBPS = [2, 5, 10, 20, 50] as const;

const BYTES_PER_MBIT = 125_000;

export function SendSpeed() {
  const t = useT();
  const { lang } = useLang();
  const w = t.ui.video;
  const { settings, update } = useSettings();
  const bps = settings?.send_limit_bps ?? null;
  const current = bps ? Math.round(bps / BYTES_PER_MBIT) : 0;
  // A value kept from elsewhere stays choosable rather than shown as something else.
  const choices: number[] = [...SEND_MBPS];
  if (current > 0 && !choices.includes(current)) {
    choices.push(current);
    choices.sort((a, b) => a - b);
  }

  return (
    <label className="video__send-speed">
      <span>{w.sendSpeed}</span>
      <select
        value={current}
        disabled={settings === null}
        onChange={(e) => {
          const n = Number(e.target.value);
          update({ send_limit_bps: n > 0 ? n * BYTES_PER_MBIT : null });
        }}
        data-testid="send-speed"
      >
        <option value={0}>{w.sendNoLimit}</option>
        {choices.map((n) => (
          <option key={n} value={n}>
            {fill(w.sendMbps, { n }, t, lang)}
          </option>
        ))}
      </select>
    </label>
  );
}
