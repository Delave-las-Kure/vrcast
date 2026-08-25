/**
 * T100 — единый экран задач (FR-082).
 *
 * Все задачи в одном месте: и заливки, и подготовка, и всё, что появится позже.
 * Держать для каждого вида свой список значило бы заставить человека обходить
 * приложение кругом, чтобы понять, чем оно занято.
 *
 * Продвижение приходит событиями, а не опросом, — иначе показ многочасовой задачи
 * сам стал бы причиной подтормаживания (SC-009, R-15).
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import type { AppError, Task, TaskOnClose, TaskState } from "../../shared/contract";
import { ipc, onTaskDone, onTaskProgress, toAppError } from "../../shared/ipc";
import { ErrorNotice } from "../shared/ErrorNotice";
import { CloseConsequences } from "./CloseConsequences";
import { QueueOrder } from "./QueueOrder";

const STATE_LABEL: Record<TaskState, string> = {
  queued: "в очереди",
  running: "выполняется",
  paused: "приостановлена",
  completed: "завершена",
  failed: "не удалась",
  cancelled: "отменена",
};

const KIND_LABEL: Record<string, string> = {
  probe: "разбор исходника",
  convert: "подготовка файла",
  upload: "заливка на сервер",
  build_ladder: "сборка набора качеств",
  deploy: "развёртывание",
  upgrade_server: "обновление сервера",
  diagnose: "диагностика",
};

function formatSpeed(bps: number | null): string | null {
  if (bps === null || bps <= 0) return null;
  const mbit = (bps * 8) / 1_000_000;
  return `${mbit.toFixed(1)} Мбит/с`;
}

function formatEta(seconds: number | null): string | null {
  if (seconds === null || seconds <= 0) return null;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `осталось ~${h} ч ${m} мин`;
  if (m > 0) return `осталось ~${m} мин`;
  return "осталось меньше минуты";
}

export function TasksPanel() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [onClose, setOnClose] = useState<TaskOnClose[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      setTasks(await ipc.tasksList());
      setError(null);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setLoading(false);
    }
    // Последствия закрытия — отдельным запросом: их считает ядро, и повторять
    // этот счёт в интерфейсе значило бы однажды разойтись с ним в ответах.
    // Неудача здесь не ломает список задач: это подсказка, а не сам список.
    try {
      setOnClose(await ipc.tasksOnClose());
    } catch {
      setOnClose([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Продвижение приходит потоком; полный список перечитываем только на завершении,
  // когда меняется состав, а не значение показателя.
  useEffect(() => {
    // Подписка оформляется асинхронно, и размонтирование может случиться раньше,
    // чем она завершится (в dev это гарантирует StrictMode). Тогда отписку уже
    // некому вызвать из cleanup — подписку, пришедшую после него, гасим на месте,
    // иначе обработчики копятся до конца сеанса с каждым заходом в раздел.
    let cancelled = false;
    const unlisten: Array<() => void> = [];
    const keep = (fn: () => void) => {
      if (cancelled) fn();
      else unlisten.push(fn);
    };

    void onTaskProgress((e) => {
      setTasks((prev) =>
        prev.map((t) =>
          t.id === e.id
            ? {
                ...t,
                state: e.state,
                progress: e.progress,
                stage: e.stage,
                speed_bps: e.speed_bps,
                eta_s: e.eta_s,
              }
            : t,
        ),
      );
    }).then(keep);

    void onTaskDone(() => void reload()).then(keep);

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => fn());
    };
  }, [reload]);

  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
      await reload();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  // Ждущие — в том порядке, в каком они пойдут в работу, а не в порядке появления
  // в списке: иначе номера в очереди не совпали бы с тем, что делает ядро.
  const queued = useMemo(
    () =>
      tasks
        .filter((t) => t.state === "queued")
        .sort((a, b) => a.queue_order - b.queue_order),
    [tasks],
  );

  if (loading) return <div className="panel">Читаем список задач…</div>;

  return (
    <div className="panel">
      <h1>Задачи</h1>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <CloseConsequences items={onClose} />

      <QueueOrder
        queued={queued}
        busy={busy}
        onReorder={(ids) => void act(async () => void (await ipc.tasksReorder(ids)))}
      />

      {tasks.length === 0 ? (
        <p className="muted">
          Задач пока нет. Они появятся, когда вы начнёте готовить или заливать видео.
        </p>
      ) : (
        <ul className="task-list">
          {tasks.map((t) => (
            <li key={t.id} className={`task task--${t.state}`}>
              <div className="task__head">
                <span className="task__kind">{KIND_LABEL[t.kind] ?? t.kind}</span>
                <span className="task__state">{STATE_LABEL[t.state]}</span>
              </div>

              {(t.state === "running" || t.state === "paused") && (
                <div
                  className="progress"
                  role="progressbar"
                  aria-valuenow={Math.round(t.progress * 100)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <div className="progress__fill" style={{ width: `${t.progress * 100}%` }} />
                </div>
              )}

              <div className="task__meta">
                {t.stage && <span>{t.stage}</span>}
                {formatSpeed(t.speed_bps) && <span>{formatSpeed(t.speed_bps)}</span>}
                {formatEta(t.eta_s) && <span>{formatEta(t.eta_s)}</span>}
              </div>

              {t.error && <p className="task__error">{t.error}</p>}

              <div className="task__actions">
                {t.state === "running" && (
                  <button onClick={() => void act(() => ipc.taskPause(t.id))}>
                    Приостановить
                  </button>
                )}
                {t.state === "paused" && (
                  <button onClick={() => void act(() => ipc.taskResume(t.id))}>
                    Продолжить
                  </button>
                )}
                {(t.state === "running" ||
                  t.state === "paused" ||
                  t.state === "queued") && (
                  <button
                    className="button--danger"
                    onClick={() => void act(() => ipc.taskCancel(t.id))}
                  >
                    Отменить
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
