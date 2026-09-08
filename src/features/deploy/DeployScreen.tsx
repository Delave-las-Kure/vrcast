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
import { toInput } from "../servers/EditServerDialog";
import { useServerById } from "../servers/store";
import { useT } from "../../shared/i18n";
import { ipc, onDeployProgress, onTaskDone } from "../../shared/ipc";
import type {
  AppError,
  DeployPreview,
  DomainAnswer,
  Ipv6Choice as Choice,
  Ipv6Mode,
  PlannedStep,
} from "../../shared/contract";

/** `Choice` (this screen, `"Keep" | "Disable"`) to `Ipv6Mode` (the profile,
 *  `"keep" | "disable"`) — two different serialisations of the same two-way decision,
 *  kept apart deliberately (T525(3)): the deploy command and the profile are different
 *  parts of the contract, and reading one field's exact spelling off the other would be
 *  the guess the core's own comment warns against. */
function choiceToMode(choice: Choice): Ipv6Mode {
  return choice === "Keep" ? "keep" : "disable";
}

export function DeployScreen({ serverId }: { serverId: string }) {
  const t = useT();
  const words = t.ui.deploy;
  const profile = useServerById(serverId);

  // **`null` means nobody has chosen yet** (T525(1)). `deploy/ipv6.rs` in the core says it
  // plainly in its own header: two paths, and neither of them is a default — a default here
  // would be a silent decision about somebody else's viewers. There used to be one
  // ("Disable"), which is exactly the decision the core's comment warns against making for
  // a person.
  const [ipv6, setIpv6] = useState<Choice | null>(null);
  const [domain, setDomain] = useState<DomainAnswer | null>(null);
  const [preview, setPreview] = useState<DeployPreview | null>(null);
  const [live, setLive] = useState<PlannedStep[] | null>(null);
  /** The id of the running task, not a flag: other tasks run alongside, and somebody else's
   *  ending is not ours. */
  const [running, setRunning] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  const domainOk = domain !== null && domain.advice === null;
  // **The button waits on both** (T525(1)). `domainOk` alone would still let a person start
  // with no IPv6 choice made at all if the domain happened to check out before one is picked
  // — impossible today since `DomainCheck` only runs once `ipv6` is chosen, but the
  // condition says the real rule rather than leaning on that ordering.
  const readyToStart = ipv6 !== null && domainOk;

  // **The choice is remembered, not just used for this run** (T525(3)). Without this, the
  // profile keeps `ipv6_mode: null` forever — `SetupWizard` rightly writes `null` at
  // creation time, since nobody has seen this screen yet, but from here on a person who
  // reopens this screen (or restarts the application) should not be asked the same question
  // with no memory of the last answer. A failed write is not allowed to block the
  // deployment itself: it is a side effect of the choice, not a precondition for using it —
  // so it is attempted silently and its failure does not surface as an `ErrorNotice`,
  // exactly as it would not stop `start` below from running with the in-memory `ipv6`.
  const chooseIpv6 = useCallback(
    (choice: Choice) => {
      setIpv6(choice);
      if (!profile) return;
      const input = { ...toInput(profile), ipv6_mode: choiceToMode(choice) };
      void ipc.serverUpdate(serverId, input, null).catch(() => {
        // Silent: see the comment above `chooseIpv6`. The person's choice still drives
        // this run of the deploy screen either way.
      });
    },
    [profile, serverId],
  );


  // The plan is asked for only once the domain is right. Asking earlier is possible, but a
  // list of changes that cannot be applied reads as an offer — a person agrees to it, and
  // still nothing can start.
  useEffect(() => {
    if (ipv6 === null || !domainOk || running) return;
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
    if (ipv6 === null) return;
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
      <Ipv6Choice value={ipv6} onChange={chooseIpv6} disabled={running !== null} />

      {/* Nothing to check the domain against until a choice is made: which record has to
          exist is exactly what the choice decides. */}
      {ipv6 !== null && <DomainCheck serverId={serverId} ipv6={ipv6} onAnswer={setDomain} />}

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
          <button type="button" onClick={start} disabled={!readyToStart}>
            {words.agreeAndStart}
          </button>
        </>
      )}
    </section>
  );
}
