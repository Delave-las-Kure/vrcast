/**
 * T322 — the mascot itself, drawn in code.
 *
 * **The owner's decision of 2026-08-27:** there will be no Rive file. It is binary, it is
 * drawn in somebody else's editor, and without it the state machine would have stayed empty
 * and the phase unfinished. What is here is a figure made of a few shapes, with the moods done
 * as CSS transitions.
 *
 * What that won besides time: **no package, no line in THIRD-PARTY, no rendering engine**. The
 * smoothness measurement on Linux that R-16 wanted at the start of the phase was about the
 * Rive engine — the system graphics there are weaker, and the mascot might have cost more than
 * it is worth. Half a dozen shapes with opacity and offset transitions do not carry that cost
 * on any of the platforms, so the reason to have it off by default went away with the engine.
 * The "turn it off" setting stayed, and it turns it off for real (T328).
 *
 * **This file is fetched on demand.** That is exactly why it is a file of its own: with the
 * mascot off it is never asked for — see `Mascot.tsx`.
 *
 * Movement is stopped by `prefers-reduced-motion` and by the setting alike (FR-103). That is
 * checked in the CSS rather than here: a rule living beside the animation cannot drift from
 * it.
 */

import type { Mood } from "./state";

export default function MascotDrawing({ mood, label }: { mood: Mood; label: string }) {
  return (
    <svg
      className={`mascot mascot--${mood}`}
      viewBox="0 0 64 64"
      role="img"
      aria-label={label}
      data-mood={mood}
      data-testid="mascot-drawing"
    >
      {/* The body. The colours come from the same variables as everything else: a mascot that
          does not change with the dark theme glows as a white blob on a dark screen. */}
      <circle className="mascot__body" cx="32" cy="36" r="20" />
      {/* The antenna, which is what shows work: still at rest, swaying while working. */}
      <line className="mascot__antenna" x1="32" y1="16" x2="32" y2="6" />
      <circle className="mascot__spark" cx="32" cy="5" r="3.5" />
      {/* The eyes. The difference between the moods is in them: narrowed on success, wide
          open on trouble. The shape is changed by CSS, so that one and the same tree is not
          rebuilt. */}
      <circle className="mascot__eye mascot__eye--left" cx="25" cy="33" r="3" />
      <circle className="mascot__eye mascot__eye--right" cx="39" cy="33" r="3" />
      {/* The mouth. An arc where only the curve changes. */}
      <path className="mascot__mouth" d="M24 43 Q32 48 40 43" fill="none" />
    </svg>
  );
}
