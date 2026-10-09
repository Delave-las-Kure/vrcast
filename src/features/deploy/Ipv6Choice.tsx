/**
 * T292 — keep IPv6 or turn it off (FR-135).
 *
 * **A question, not a default.** The choice changes two things at once: which DNS records have
 * to exist, and whether viewers whose connection prefers IPv6 will see the stream at all. A
 * silent default here is a decision about other people's viewers, taken for a person and
 * without them.
 *
 * What each path costs is written beside the path itself rather than hidden in a tooltip:
 * somebody buying a server for the first time does not know what AAAA is, and here is where
 * they should find out.
 *
 * T709: each option on a line of its own, its cost under its name. They used to run inline
 * into one another — «Оставить IPv6Нужна запись AAAA… Отключить IPv6Запись AAAA…» — and which
 * explanation belonged to which button was a guess.
 */

import { useT } from "../../shared/i18n";
import type { Ipv6Choice as Choice } from "../../shared/contract";

export function Ipv6Choice({
  value,
  onChange,
  disabled,
}: {
  /** `null` means nobody has chosen yet — neither option is the default (T525(1)). */
  value: Choice | null;
  onChange: (choice: Choice) => void;
  disabled?: boolean;
}) {
  const t = useT();
  const words = t.ui.deploy;

  return (
    <fieldset className="choices">
      <legend>{words.ipv6Question}</legend>

      {/* Neither radio is checked while `value` is `null` — the reader has to say why the
          start button below is disabled, because the fieldset alone looks like a choice was
          simply left where it started. */}
      {value === null && <p>{words.ipv6NotChosen}</p>}

      <label className="choice">
        <input
          type="radio"
          name="ipv6"
          value="Keep"
          checked={value === "Keep"}
          disabled={disabled}
          onChange={() => onChange("Keep")}
        />
        <span className="choice__text">
          <strong>{words.ipv6Keep}</strong>
          <span className="choice__means">{words.ipv6KeepMeans}</span>
        </span>
      </label>

      <label className="choice">
        <input
          type="radio"
          name="ipv6"
          value="Disable"
          checked={value === "Disable"}
          disabled={disabled}
          onChange={() => onChange("Disable")}
        />
        <span className="choice__text">
          <strong>{words.ipv6Disable}</strong>
          <span className="choice__means">{words.ipv6DisableMeans}</span>
        </span>
      </label>
    </fieldset>
  );
}
