/**
 * T052 — the server setup wizard.
 *
 * Three steps, and the order between them is not optional:
 *
 * 1. **Details.** A form of profile fields. Every objection is shown at once — a
 *    person fills the form in whole, and sending them round again for each typo is
 *    work that need not exist.
 * 2. **Fingerprint.** The application learns the server's fingerprint **presenting it
 *    nothing**, and shows it. Until it is confirmed, neither password nor key goes to
 *    the server (FR-092). This is the one step that cannot be skipped.
 * 3. **Check.** Four steps in a row: network, sign-in, video directory, serving over
 *    the domain. All are shown, marked with where it stopped (FR-003).
 *
 * If a `server.env` from the old way of working is found nearby, the fields can be
 * filled in from it (T043). The file is only read, never changed.
 */

import { useEffect, useState } from "react";
import type { AppError, ImportSuggestion, ServerInput, TestStep } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { renderDetail } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";
import { ServerForm } from "./ServerForm";
import { useServers } from "./store";

type Stage = "form" | "fingerprint" | "test" | "done";

const EMPTY: ServerInput = {
  name: "",
  host: "",
  port: 22,
  user: "root",
  auth_kind: "key",
  key_path: null,
  domain: "",
  video_dir: null,
  cdn_base: null,
  ipv6_mode: null,
};

export function SetupWizard({ onClose }: { onClose: () => void }) {
  const reload = useServers((s) => s.reload);

  const [stage, setStage] = useState<Stage>("form");
  const [input, setInput] = useState<ServerInput>(EMPTY);
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const [serverId, setServerId] = useState<string | null>(null);
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [steps, setSteps] = useState<TestStep[] | null>(null);
  const [suggestion, setSuggestion] = useState<ImportSuggestion | null>(null);
  const t = useT();
  const w = t.ui.wizard;

  // The import suggestion is looked for once, on opening. Its absence is ordinary
  // rather than a problem: most people have no such file and never will.
  useEffect(() => {
    let cancelled = false;
    ipc
      .serverImportSuggestion()
      .then((s) => {
        if (!cancelled) setSuggestion(s);
      })
      .catch(() => {
        if (!cancelled) setSuggestion(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /** Step 1 to 2: create the profile and learn the fingerprint. */
  const submitForm = async () => {
    setBusy(true);
    setError(null);
    try {
      const id = await ipc.serverAdd(input, secret);
      setServerId(id);
      // The secret is no longer needed in the interface's memory: it went into the
      // system store and is never handed back.
      setSecret("");
      setFingerprint(await ipc.serverProbeFingerprint(input.host, input.port));
      setStage("fingerprint");
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /** Step 2 to 3: confirm the fingerprint and run the check. */
  const confirmFingerprint = async () => {
    if (!serverId || !fingerprint) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.serverFingerprintConfirm(serverId, fingerprint);
      setStage("test");
      setSteps(await ipc.serverTest(serverId));
      await reload();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /** Giving up at the fingerprint step: the profile exists already and must not stay. */
  const abandon = async () => {
    if (serverId) {
      try {
        await ipc.serverRemove(serverId);
      } catch {
        // Cleaning up failed — it will show in the list and can be deleted by hand.
        // Closing must not be held up for it: the person has already decided.
      }
      await reload();
    }
    onClose();
  };

  const finish = async () => {
    if (serverId) await useServers.getState().setActive(serverId);
    onClose();
  };

  return (
    <div className="wizard" role="dialog" aria-label={w.dialogLabel}>
      <header className="wizard__head">
        <h2>{w.heading}</h2>
        <ol className="wizard__steps">
          <li className={stage === "form" ? "is-current" : "is-done"}>{w.stepData}</li>
          <li
            className={stage === "fingerprint" ? "is-current" : stage === "form" ? "" : "is-done"}
          >
            {w.stepFingerprint}
          </li>
          <li className={stage === "test" || stage === "done" ? "is-current" : ""}>{w.stepTest}</li>
        </ol>
      </header>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {stage === "form" && (
        <>
          {suggestion && (
            <div className="notice" role="status">
              <div className="notice__body">
                <strong className="notice__message">{w.importFound}</strong>
                <p className="notice__hint">
                  {suggestion.source} {w.importExplain}
                  {suggestion.needs_passphrase && w.importNeedsPassphrase}
                </p>
              </div>
              <button
                onClick={() => {
                  setInput(suggestion.input);
                  setSuggestion(null);
                }}
              >
                {w.importApply}
              </button>
            </div>
          )}

          <ServerForm
            input={input}
            onFieldChange={(key, value) => setInput((prev) => ({ ...prev, [key]: value }))}
            secret={secret}
            onSecretChange={setSecret}
            secretHint={w.secretHint}
            busy={busy}
            submitLabel={w.next}
            busyLabel={w.checking}
            onSubmit={() => void submitForm()}
            onCancel={onClose}
          />
        </>
      )}

      {stage === "fingerprint" && fingerprint && (
        <section className="wizard__stage">
          <p>{w.fingerprintLead}</p>
          <code className="fingerprint">{fingerprint}</code>
          <p className="muted">{w.fingerprintWhy}</p>
          <div className="form__actions">
            <button type="button" onClick={() => void abandon()} disabled={busy}>
              {w.abandon}
            </button>
            <button type="button" onClick={() => void confirmFingerprint()} disabled={busy}>
              {busy ? w.checking : w.fingerprintOk}
            </button>
          </div>
        </section>
      )}

      {stage === "test" && (
        <section className="wizard__stage">
          <TestSteps steps={steps} />
          <div className="form__actions">
            <button
              type="button"
              onClick={() => {
                if (!serverId) return;
                setBusy(true);
                ipc
                  .serverTest(serverId)
                  .then(setSteps)
                  .catch((e) => setError(toAppError(e)))
                  .finally(() => setBusy(false));
              }}
              disabled={busy}
            >
              {w.testAgain}
            </button>
            <button type="button" onClick={() => void finish()} disabled={busy}>
              {w.done}
            </button>
          </div>
        </section>
      )}
    </div>
  );
}

/**
 * Showing the steps of the check.
 *
 * **Every** step is shown, including the ones not attempted: a person needs to see
 * what got through, not only the last thing that went wrong (FR-003).
 *
 * The title comes from the step's id rather than travelling with it: the core stopped
 * composing prose, and one catalogue means one wording per step on every screen.
 */
export function TestSteps({ steps }: { steps: TestStep[] | null }) {
  const t = useT();
  const { lang } = useLang();

  if (!steps) return <p className="muted">{t.ui.wizard.testRunning}</p>;

  return (
    <ol className="steps">
      {steps.map((step) => (
        <li key={step.id} className={`step step--${step.status}`}>
          {/*
            The mark says the status in a glyph, and the glyph was hidden from a screen
            reader with nothing put in its place — so somebody listening was told which
            steps there were and never which of them passed. The words existed in both
            catalogues and were shown nowhere.
          */}
          <span className="step__mark" aria-hidden="true">
            {step.status === "ok" ? "✓" : step.status === "failed" ? "✕" : "·"}
          </span>
          <span className="visually-hidden">
            {t.ui.servers.stepStatus[step.status as keyof typeof t.ui.servers.stepStatus] ??
              step.status}
          </span>
          <div className="step__body">
            <span className="step__title">
              {t.ui.servers.steps[step.id as keyof typeof t.ui.servers.steps] ?? step.id}
            </span>
            {step.detail && (
              <span className="step__detail">{renderDetail(step.detail, t, lang)}</span>
            )}
            {step.status === "skipped" && !step.detail && (
              <span className="step__detail muted">{t.ui.wizard.stepSkipped}</span>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
