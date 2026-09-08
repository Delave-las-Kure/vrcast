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
    <fieldset>
      <legend>{words.ipv6Question}</legend>

      {/* Neither radio is checked while `value` is `null` — the reader has to say why the
          start button below is disabled, because the fieldset alone looks like a choice was
          simply left where it started. */}
      {value === null && <p>{words.ipv6NotChosen}</p>}

      <label>
        <input
          type="radio"
          name="ipv6"
          value="Keep"
          checked={value === "Keep"}
          disabled={disabled}
          onChange={() => onChange("Keep")}
        />
        <strong>{words.ipv6Keep}</strong>
        <span>{words.ipv6KeepMeans}</span>
      </label>

      <label>
        <input
          type="radio"
          name="ipv6"
          value="Disable"
          checked={value === "Disable"}
          disabled={disabled}
          onChange={() => onChange("Disable")}
        />
        <strong>{words.ipv6Disable}</strong>
        <span>{words.ipv6DisableMeans}</span>
      </label>
    </fieldset>
  );
}
