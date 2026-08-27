/**
 * T292 — оставить IPv6 или отключить (FR-135).
 *
 * **Вопрос, а не умолчание.** Выбор меняет две вещи сразу: какие доменные записи обязаны
 * существовать и увидят ли раздачу зрители, чьё соединение предпочитает IPv6. Молчаливое
 * умолчание здесь — это решение про чужих зрителей, принятое за человека и без него.
 *
 * Последствия обоих путей написаны рядом с самими путями, а не спрятаны в подсказку: тот,
 * кто впервые купил сервер, не знает, что такое AAAA, и узнать это должен здесь.
 */

import { useT } from "../../shared/i18n";
import type { Ipv6Choice as Choice } from "../../shared/contract";

export function Ipv6Choice({
  value,
  onChange,
  disabled,
}: {
  value: Choice;
  onChange: (choice: Choice) => void;
  disabled?: boolean;
}) {
  const t = useT();
  const words = t.ui.deploy;

  return (
    <fieldset>
      <legend>{words.ipv6Question}</legend>

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
