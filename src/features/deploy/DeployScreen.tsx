/**
 * T291 — deploying a server (FR-121, FR-122, FR-123).
 *
 * Three things, and each of them a promise made by milestone D.
 *
 * **Nothing happens until a person has seen the list and agreed to it** (FR-122). Not "some
 * packages will be installed", but by name: which user, which directories, which ports will be
 * opened, which kernel settings will change. This is somebody else's machine, and its owner has
 * a right to know what will be done to it.
 *
 * **Progress is shown step by step** (FR-123). Four minutes of "deploying…" tells a person
 * nothing about whether to wait or to go and fix a DNS record. The steps arrive in an event
 * carrying the **whole** list: a screen that assembled it from a stream of single ones would
 * show something different the moment one went missing — and one will, because people open the
 * screen halfway through.
 *
 * **A failed step is named.** "Deployment failed" and "the firewall step failed" send a person
 * to two different places, and only one of them is a place.
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
  /** The id of the running task, not a flag: other tasks run alongside, and somebody else's
   *  ending is not ours. */
  const [running, setRunning] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const domainOk = domain !== null && domain.advice === null;

  // The plan is asked for only once the domain is right. Asking earlier is possible, but a
  // list of changes that cannot be applied reads as an offer — a person agrees to it, and
  // still nothing can start.
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

  // How the deployment is going.
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

      {/* The choice comes first: which DNS record has to exist depends on it. */}
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
          {/* About the machine, because two steps depend on it: a small one gets a swap file,
              and a person has a right to know such a file will appear on their server. */}
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
