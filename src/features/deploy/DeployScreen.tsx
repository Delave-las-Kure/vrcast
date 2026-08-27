/**
 * T291 — развернуть сервер (FR-121, FR-122, FR-123).
 *
 * Три вещи, и каждая из них — обещание вехи D.
 *
 * **Ничего не происходит, пока человек не увидел список и не согласился** (FR-122). Список —
 * не «будет установлено несколько пакетов», а поимённо: какой пользователь, какие каталоги,
 * какие порты откроются, какие настройки ядра поменяются. Это чужая для нас машина, и её
 * хозяин вправе знать, что с ней сделают.
 *
 * **Ход показывается по шагам** (FR-123). «Разворачиваю…» четыре минуты не говорит человеку
 * ничего о том, ждать ему или пойти поправить доменную запись. Шаги приходят событием со
 * **всем** списком: экран, собирающий его из потока одиночных, покажет другое, если один
 * пропустит, — а он пропустит, потому что человек открывает экран посередине.
 *
 * **Проваленный шаг назван.** «Развёртывание не удалось» и «не удался шаг с файрволом» ведут
 * человека в разные места, и только одно из них — место.
 */

import { useCallback, useEffect, useState } from "react";

import { DomainCheck } from "./DomainCheck";
import { Ipv6Choice } from "./Ipv6Choice";
import { StepList } from "./StepList";
import { ErrorNotice } from "../shared/ErrorNotice";
import { useT } from "../../shared/i18n";
import { ipc, onDeployProgress, onTaskDone } from "../../shared/ipc";
import type {
  AppError,
  DeployPreview,
  DomainAnswer,
  Ipv6Choice as Choice,
  PlannedStep,
} from "../../shared/contract";

export function DeployScreen({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.deploy;

  const [ipv6, setIpv6] = useState<Choice>("Disable");
  const [domain, setDomain] = useState<DomainAnswer | null>(null);
  const [preview, setPreview] = useState<DeployPreview | null>(null);
  const [live, setLive] = useState<PlannedStep[] | null>(null);
  /** Номер идущей задачи, а не признак: рядом могут идти чужие, и чужой конец не наш. */
  const [running, setRunning] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const domainOk = domain !== null && domain.advice === null;

  // План спрашивается только когда домен в порядке. Спросить раньше можно, но список
  // изменений, который нельзя применить, читается как предложение — и человек согласится
  // с ним, а начать всё равно не выйдет.
  useEffect(() => {
    if (!domainOk || running) return;
    let alive = true;
    setError(null);
    ipc
      .deployPlan(serverId, ipv6)
      .then((got) => {
        if (alive) setPreview(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, [serverId, ipv6, domainOk, running]);

  // Ход развёртывания.
  useEffect(() => {
    if (!running) return;
    let alive = true;
    const steps = onDeployProgress((id, settled) => {
      if (alive && id === serverId) setLive(settled);
    });
    const finish = onTaskDone((event) => {
      if (!alive || event.id !== running) return;
      setRunning(null);
      if (event.error) setError(event.error);
      else setDone(true);
    });
    return () => {
      alive = false;
      void steps.then((off) => off());
      void finish.then((off) => off());
    };
  }, [running, serverId]);

  const start = useCallback(() => {
    setError(null);
    setLive(null);
    ipc
      .deployRun(serverId, ipv6, true)
      .then(setRunning)
      .catch((e: AppError) => setError(e));
  }, [serverId, ipv6]);

  if (done) {
    return (
      <section aria-label={words.title}>
        <h2>{words.title}</h2>
        <p>{words.finished}</p>
      </section>
    );
  }

  return (
    <section aria-label={words.title}>
      <h2>{words.title}</h2>

      {error && <ErrorNotice error={error} />}

      {/* Выбор идёт первым: от него зависит, какая доменная запись обязана быть. */}
      <Ipv6Choice value={ipv6} onChange={setIpv6} disabled={running !== null} />

      <DomainCheck serverId={serverId} ipv6={ipv6} onAnswer={setDomain} />

      {running !== null && (
        <>
          <p>{words.running}</p>
          <StepList steps={live ?? preview?.steps ?? []} />
        </>
      )}

      {running === null && preview && (
        <>
          <h3>{words.willChange}</h3>
          {/* Про машину — потому что от неё зависят два шага: маленькой делается файл
              подкачки, и человек вправе знать, что на его сервере появится такой файл. */}
          <p>{words.machine(preview.memory_mb, preview.disk)}</p>
          <StepList steps={preview.steps} />
          <button type="button" onClick={start} disabled={!domainOk}>
            {words.agreeAndStart}
          </button>
        </>
      )}
    </section>
  );
}
