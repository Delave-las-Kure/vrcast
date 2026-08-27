/**
 * T317 — экран диагностики (FR-070, FR-071, FR-072).
 *
 * **Порядок разделов — это метод, а не вёрстка.** Сначала состояние сервера: спит он или
 * работает. Потом журнал: что раздача вообще делала. Потом разбор подвисаний: почему у этого
 * зрителя встаёт. И только потом файл — на отдельной вкладке, потому что до него доходят
 * последними. Перевёрнутый порядок — это вечер, потраченный на перекодирование фильма ради
 * чужого Wi-Fi.
 *
 * **«Не удалось определить» — отдельное состояние, а не пустой экран.** Пустое место человек
 * читает как «всё хорошо» или как поломку приложения; и то и другое неправда, а разница между
 * ними — это разница между «идти чинить сервер» и «спросить ещё раз».
 *
 * Ничего здесь сервер не меняет. Все четыре вопроса — на чтение, и потому их можно задать и
 * чужой машине, и той, что новее этого приложения: смотреть человеку нужно именно там.
 */

import { useCallback, useEffect, useState } from "react";

import { BitratePeaks } from "./BitratePeaks";
import { HealthPanel } from "./HealthPanel";
import { LogsPanel } from "./LogsPanel";
import { StallsPanel } from "./StallsPanel";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import type { AppError, Health, Logs, Stalls } from "../../shared/contract";

/** За сколько минут спрашивать журнал по умолчанию. */
const DEFAULT_MINUTES = 30;

const PERIODS = [10, 30, 60, 120];

export function DiagScreen({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.diag;

  const [minutes, setMinutes] = useState(DEFAULT_MINUTES);
  const [health, setHealth] = useState<Health | null>(null);
  const [logs, setLogs] = useState<Logs | null>(null);
  const [stalls, setStalls] = useState<Stalls | null>(null);
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const ask = useCallback(async () => {
    setAsking(true);
    setError(null);
    try {
      // По очереди, а не разом: соединение одно, и три вопроса, поданных вместе, займут три
      // канала из восьми — два из которых уже держит слежение за зрителями (R-04). А разбор
      // подвисаний ещё и меряет живую нагрузку пять секунд, и мерить её, пока рядом идут
      // наши же вопросы, значит мерить себя.
      setHealth(await ipc.diagHealth(serverId));
      setLogs(await ipc.diagLogs(serverId, minutes));
      setStalls(await ipc.diagExplainStalls(serverId, minutes));
    } catch (e) {
      setError(e as AppError);
    } finally {
      setAsking(false);
    }
  }, [serverId, minutes]);

  useEffect(() => {
    void ask();
  }, [ask]);

  const nothingCameBack = !asking && !error && health === null;

  return (
    <div className="diag-screen">
      <h2>{words.title}</h2>

      <div className="diag-controls">
        <label>
          {words.period}{" "}
          <select
            value={minutes}
            onChange={(e) => setMinutes(Number(e.target.value))}
            data-testid="diag-period"
          >
            {PERIODS.map((m) => (
              <option key={m} value={m}>
                {m} {words.minutes}
              </option>
            ))}
          </select>
        </label>
        <button type="button" onClick={() => void ask()} disabled={asking}>
          {words.refresh}
        </button>
      </div>

      {asking && <p data-testid="diag-asking">{words.asking}</p>}
      {error && <ErrorNotice error={error} />}
      {nothingCameBack && <p data-testid="diag-nothing">{words.notDetermined}</p>}

      {health && <HealthPanel health={health} />}
      {logs && <LogsPanel logs={logs} />}
      {stalls && <StallsPanel stalls={stalls} />}

      <BitratePeaks />
    </div>
  );
}
