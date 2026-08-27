/**
 * T294 — что это за сервер, прямо в его карточке (FR-120, FR-128, FR-130, FR-132).
 *
 * Пять состояний, и каждое ведёт человека в своё место. Общее «что-то не так» не ведёт
 * никуда:
 *
 * - **чистый** — предложить развернуть;
 * - **незакончено** — предложить довести. Это наша собственная прерванная работа, и назвать
 *   её чужим сервером значит отказаться доводить своё же;
 * - **наш** — версия серверной части рядом с версией приложения. Порознь они не значат
 *   ничего: «серверная часть 1» — это число, пока не сказано, какую разворачивает приложение;
 * - **новее, чем мы понимаем** — предупредить и ничего не менять (FR-130). Приложение
 *   постарше, записывающее файлы туда, где новая раскладка их не держит, — это способ тихо
 *   сломать работающий сервер;
 * - **чужой** — сказать, **что именно** распознано (FR-132). «Посторонняя настройка» — это
 *   не то, с чем можно пойти и разобраться.
 */

import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { UpgradeDialog } from "./UpgradeDialog";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type { AppError, ServerState } from "../../shared/contract";

export function ServerStateCard({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.serverState;

  const [state, setState] = useState<ServerState | null>(null);
  const [asking, setAsking] = useState(true);
  const [error, setError] = useState<AppError | null>(null);
  const [upgrading, setUpgrading] = useState(false);

  useEffect(() => {
    let alive = true;
    setAsking(true);
    ipc
      .serverDetect(serverId)
      .then((got) => {
        if (alive) setState(got);
      })
      .catch((e: AppError) => {
        // Сервер, который не ответил, — не поломка приложения. Показываем причину и не
        // делаем вид, что состояние известно.
        if (alive) setError(e);
      })
      .finally(() => {
        if (alive) setAsking(false);
      });
    return () => {
      alive = false;
    };
  }, [serverId, upgrading]);

  if (asking) return <p>{words.asking}</p>;
  if (error) return <ErrorNotice error={error} />;
  if (!state) return null;

  if (upgrading) {
    return <UpgradeDialog serverId={serverId} onDone={() => setUpgrading(false)} onCancel={() => setUpgrading(false)} />;
  }

  return (
    <div aria-label={words.title}>
      {state.kind === "Clean" && (
        <>
          <p>{words.clean}</p>
          <Link to={`/deploy?server=${encodeURIComponent(serverId)}`}>{words.deployIt}</Link>
        </>
      )}

      {state.kind === "Unfinished" && (
        <>
          <p>{words.unfinished}</p>
          <Link to={`/deploy?server=${encodeURIComponent(serverId)}`}>{words.finishIt}</Link>
        </>
      )}

      {state.kind === "Managed" && (
        <>
          {/* Рядом, а не порознь. */}
          <p>{words.versions(state.server_version ?? 0, state.app_expects)}</p>
          {state.compat === "TooNew" && <p>{words.tooNew}</p>}
          {(state.upgrade_available || state.compat === "NeedsUpgrade") && (
            <button type="button" onClick={() => setUpgrading(true)}>
              {words.updateIt}
            </button>
          )}
        </>
      )}

      {state.kind === "Foreign" && (
        <>
          <p>{words.foreign}</p>
          {/* Что именно найдено. Без этого отказ не с чем связать. */}
          {state.foreign_reason !== null && <small>{JSON.stringify(state.foreign_reason)}</small>}
        </>
      )}

      {state.kind === "Unreachable" && <p>{words.unreachable}</p>}
    </div>
  );
}
