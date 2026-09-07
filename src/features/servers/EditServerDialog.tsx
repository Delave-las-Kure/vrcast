/**
 * T492 — editing a server profile (FR-005).
 *
 * The core's `serverUpdate` already existed; nothing here talked to it. The one wrinkle
 * that matters: `host`/`port` are what the fingerprint was pinned to, and the core's
 * dangerous part (moving the fingerprint on an address change) has already been cleaned
 * up there — the screen's job is only to notice a change happened and walk the person
 * through re-confirming it, the same way `SetupWizard` does on first add.
 *
 * Stages:
 *
 * 1. **Form.** The same fields as adding a server, filled with what the profile already
 *    has. The secret field is never pre-filled — the core never hands one back — and
 *    stays empty with a note that emptiness means "leave it as it is" (`ipc.serverUpdate`
 *    only replaces the secret when one is given).
 * 2. **Fingerprint.** Only reached when `host` or `port` changed. `serverUpdate` has
 *    already gone through by this point; what is shown here is the *new* address's
 *    fingerprint, exactly as on first add, and it must be confirmed before anything
 *    connects to it (FR-092).
 * 3. **Test.** The ordinary connection check, shown the same way `TestSteps` shows it
 *    everywhere else.
 *
 * When neither `host` nor `port` changed, saving stops after the update: the confirmed
 * fingerprint from before still applies, and re-probing it would be asking a question
 * that was already answered.
 */

import { useState } from "react";
import type { AppError, ServerInput, ServerProfile, TestStep } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useLang, useT } from "../../shared/i18n";
import { fill } from "../../shared/i18n/render";
import { ErrorNotice } from "../shared/ErrorNotice";
import { ServerForm } from "./ServerForm";
import { TestSteps } from "./SetupWizard";
import { useServers } from "./store";

type Stage = "form" | "fingerprint" | "test";

function toInput(profile: ServerProfile): ServerInput {
  return {
    name: profile.name,
    host: profile.host,
    port: profile.port,
    user: profile.user,
    auth_kind: profile.auth_kind,
    key_path: profile.key_path,
    domain: profile.domain,
    video_dir: profile.video_dir,
    cdn_base: profile.cdn_base,
    ipv6_mode: profile.ipv6_mode,
  };
}

export function EditServerDialog({
  profile,
  onClose,
}: {
  profile: ServerProfile;
  onClose: () => void;
}) {
  const reload = useServers((s) => s.reload);

  const [stage, setStage] = useState<Stage>("form");
  const [input, setInput] = useState<ServerInput>(() => toInput(profile));
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [steps, setSteps] = useState<TestStep[] | null>(null);
  const t = useT();
  const { lang } = useLang();
  const s = t.ui.servers;
  const w = t.ui.wizard;

  /** Step 1: save the changes. Re-checks the connection only if `host`/`port` moved. */
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await ipc.serverUpdate(profile.id, input, secret === "" ? null : secret);
      setSecret("");
      const addressChanged = input.host !== profile.host || input.port !== profile.port;
      if (addressChanged) {
        setFingerprint(await ipc.serverProbeFingerprint(input.host, input.port));
        setStage("fingerprint");
      } else {
        await reload();
        onClose();
      }
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /** Step 2: confirm the new address's fingerprint and run the check. */
  const confirmFingerprint = async () => {
    if (!fingerprint) return;
    setBusy(true);
    setError(null);
    try {
      await ipc.serverFingerprintConfirm(profile.id, fingerprint);
      setStage("test");
      setSteps(await ipc.serverTest(profile.id));
      await reload();
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    await reload();
    onClose();
  };

  const heading = fill(s.editHeading, { name: profile.name }, t, lang);

  return (
    <div className="wizard" role="dialog" aria-label={heading}>
      <header className="wizard__head">
        <h2>{heading}</h2>
      </header>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {stage === "form" && (
        <ServerForm
          input={input}
          onFieldChange={(key, value) => setInput((prev) => ({ ...prev, [key]: value }))}
          secret={secret}
          onSecretChange={setSecret}
          secretHint={s.editSecretHint}
          busy={busy}
          submitLabel={s.save}
          busyLabel={s.saving}
          onSubmit={() => void save()}
          onCancel={onClose}
        />
      )}

      {stage === "fingerprint" && fingerprint && (
        <section className="wizard__stage">
          <p>{s.editAddressChanged}</p>
          <p>{w.fingerprintLead}</p>
          <code className="fingerprint">{fingerprint}</code>
          <p className="muted">{w.fingerprintWhy}</p>
          <div className="form__actions">
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
            <button type="button" onClick={() => void finish()} disabled={busy}>
              {w.done}
            </button>
          </div>
        </section>
      )}
    </div>
  );
}
