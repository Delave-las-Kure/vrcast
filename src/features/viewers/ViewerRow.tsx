/**
 * T175 — one viewer in the list, and what is wrong with them.
 *
 * Two rules run through this file, and both are about not saying more than is known.
 *
 * Whatever the core could not determine is shown as **not determined**, in those words. It
 * is never left blank and never guessed at from a neighbour: a city invented from the
 * nearest range looks exactly like knowledge, and a person acts on it (FR-052).
 *
 * A viewer in trouble is marked **with the reason**. A row that merely goes red says only
 * "something is wrong with somebody", which is the state the owner was already in before
 * they opened the application (FR-053).
 */

import { useLang, useT, type Lang } from "../../shared/i18n";
import { formatBitrate, formatDuration } from "../../shared/i18n/format";
import type { Viewer, ViewerProblem } from "../../shared/contract";

/** How long they have been watching, from the two times the core sends. */
function watchingFor(viewer: Viewer): number | null {
  const from = Date.parse(viewer.started_at);
  const to = Date.parse(viewer.last_seen_at);
  if (Number.isNaN(from) || Number.isNaN(to) || to < from) return null;
  return (to - from) / 1000;
}

/** Where they are, as one line — with the parts that are known and no others. */
function place(viewer: Viewer): string | null {
  const parts = [viewer.country, viewer.city].filter(Boolean) as string[];
  return parts.length > 0 ? parts.join(", ") : null;
}

/** Nothing known about where they are: one dash, «не определено» on hover (tour 0.3.0). */
const NOWHERE = "—";

/**
 * The rung as a person reads it when the screen above could not name it: `v9` is the set's
 * directory for 9 Mbit/s (T694), so «9,0 Мбит/с» rather than the bare directory. A file
 * name (a film served as one file) is shown as it is — it is what the person named it.
 */
function plainVariant(variant: string, lang: Lang): string {
  const rung = /^v(\d+)$/.exec(variant);
  return rung ? formatBitrate(Number(rung[1]) * 1_000_000, lang) : variant;
}

export function ViewerRow({
  viewer,
  mediaTitle,
  variantLabel,
  onLimit,
  limitLabel,
}: {
  viewer: Viewer;
  /** What the medium is called. Absent while the core does not yet know what is watched. */
  mediaTitle?: string;
  /** The rung as the links name it, «1080p · 9 Мбит/с». Absent when the set is not known. */
  variantLabel?: string;
  /**
   * Offer to cap this viewer's quality.
   *
   * On the row rather than anywhere else: the one thing a person wants to do about a
   * viewer who is struggling is done where they are looking at them struggling.
   */
  onLimit?: () => void;
  limitLabel?: string;
}) {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.viewers;
  // T705 (tour 0.3.0): the state agrees with the speed shown beside it. A known speed under
  // a known need is "falling behind" even if the core's list of problems has not said so —
  // «16 кбит/с, нужно 11,1 Мбит/с — в порядке» is a contradiction on one line.
  const problems: ViewerProblem[] =
    viewer.delivery_bps !== null &&
    viewer.required_bps !== null &&
    viewer.delivery_bps < viewer.required_bps &&
    !viewer.problems.includes("SlowLink")
      ? ["SlowLink", ...viewer.problems]
      : viewer.problems;
  const troubled = problems.length > 0;

  return (
    <tr className={troubled ? "viewers__row viewers__row--problem" : "viewers__row"}>
      <td className="viewers__ip">
        <code>{viewer.ip}</code>
      </td>
      <td className="viewers__place">
        {/* One dash for nothing known (tour 0.3.0): two «не определено» one under the other
            read as two separate failures. What is known is shown, and only that. */}
        {place(viewer) === null && !viewer.asn_org ? (
          <span title={words.notKnown} data-testid="viewer-place-unknown">
            {NOWHERE}
          </span>
        ) : (
          <>
            <div>{place(viewer) ?? words.notKnown}</div>
            {viewer.asn_org && <div className="viewers__provider">{viewer.asn_org}</div>}
          </>
        )}
      </td>
      <td>
        {/* Not knowing what is being watched is a real state, not a gap: a film served as
            one file leaves no record until the showing ends. Saying so beats an empty cell,
            which reads as a fault. */}
        <div>{mediaTitle ?? words.watchingUnknown}</div>
        {viewer.variant && (
          <div className="viewers__variant">
            {variantLabel ?? plainVariant(viewer.variant, lang)}
          </div>
        )}
      </td>
      <td className="viewers__speed">
        {viewer.delivery_bps === null ? (
          <span title={words.speedNotYet}>{words.notKnown}</span>
        ) : (
          formatBitrate(viewer.delivery_bps, lang)
        )}
        {viewer.required_bps !== null && (
          <div className="viewers__needs">
            {words.needs} {formatBitrate(viewer.required_bps, lang)}
          </div>
        )}
      </td>
      <td>{formatDuration(watchingFor(viewer))}</td>
      <td>
        {/* T705 (QA-26 №12): "fine" only for what is known to be in order — both the
            speed and what the quality needs measured, and the speed enough. Nothing found is
            not the same as nothing wrong: without the two figures it says so. */}
        {troubled ? (
          <ul className="viewers__problems">
            {problems.map((problem) => (
              <li key={problem} title={reasonHint(problem, words)}>
                {reason(problem, words)}
              </li>
            ))}
          </ul>
        ) : viewer.delivery_bps === null || viewer.required_bps === null ? (
          <span className="viewers__unknown" title={words.noDataHint} data-testid="viewer-no-data">
            {words.noData}
          </span>
        ) : (
          <span className="viewers__fine">{words.fine}</span>
        )}
      </td>
      <td>
        {onLimit && (
          <button type="button" onClick={onLimit} data-testid={`limit-${viewer.ip}`}>
            {limitLabel}
          </button>
        )}
      </td>
    </tr>
  );
}

type Words = ReturnType<typeof useT>["ui"]["viewers"];

function reason(problem: ViewerProblem, words: Words): string {
  switch (problem) {
    case "SlowLink":
      return words.problems.slowLink;
    case "Retransmits":
      return words.problems.retransmits;
    case "Stalls":
      return words.problems.stalls;
  }
}

function reasonHint(problem: ViewerProblem, words: Words): string {
  switch (problem) {
    case "SlowLink":
      return words.problems.slowLinkHint;
    case "Retransmits":
      return words.problems.retransmitsHint;
    case "Stalls":
      return words.problems.stallsHint;
  }
}
