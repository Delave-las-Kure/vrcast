/**
 * T492 — the profile fields, shared by the wizard's first step and the edit screen.
 *
 * This used to be roughly 400 lines of JSX living only inside `SetupWizard`, written
 * for creating a profile. Editing needed the very same fields, and copying them into a
 * second dialogue was the one thing not to do: two copies of one form are two chances
 * for them to drift apart, silently, the next time a field is added to one and not the
 * other. So the fields moved here, and both `SetupWizard` (stage "form") and
 * `EditServerDialog` render this component instead of their own markup.
 *
 * What is *not* shared is the meaning of the secret field: on creation it is the secret
 * outright, on editing an empty field means "leave it as it is" (the core never hands a
 * stored secret back to the interface — there is nothing to show). That is why the
 * secret's hint is a prop rather than fixed text here: the two callers say two different
 * true things about the same field.
 */

import { open } from "@tauri-apps/plugin-dialog";
import { homeDir, join } from "@tauri-apps/api/path";
import type { ServerInput } from "../../shared/contract";
import { useT } from "../../shared/i18n";

export interface ServerFormProps {
  input: ServerInput;
  onFieldChange: <K extends keyof ServerInput>(key: K, value: ServerInput[K]) => void;
  secret: string;
  onSecretChange: (secret: string) => void;
  /** Differs between creating (secret required in practice) and editing (blank = keep). */
  secretHint: string;
  busy: boolean;
  submitLabel: string;
  busyLabel: string;
  onSubmit: () => void;
  onCancel: () => void;
}

export function ServerForm({
  input,
  onFieldChange,
  secret,
  onSecretChange,
  secretHint,
  busy,
  submitLabel,
  busyLabel,
  onSubmit,
  onCancel,
}: ServerFormProps) {
  const t = useT();
  const w = t.ui.wizard;

  /**
   * Find the private key in the file browser.
   *
   * **No filters.** A private key has no extension, and a filter is built from extensions
   * on both platforms this application runs on — `*.pem` on Windows, `*.pem` through GTK on
   * Linux — so any filter at all would hide `id_ed25519` from the person looking for it.
   *
   * The dialogue opens in `~/.ssh` where the keys are. If that directory is not there the
   * dialogue opens at home rather than refusing, which is the right answer for somebody who
   * keeps their keys elsewhere.
   */
  const pickKey = async () => {
    let start: string | undefined;
    try {
      start = await join(await homeDir(), ".ssh");
    } catch {
      start = undefined;
    }
    const chosen = await open({ multiple: false, directory: false, defaultPath: start });
    if (typeof chosen === "string") onFieldChange("key_path", chosen);
  };

  return (
    <form
      className="form"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit();
      }}
    >
      <label>
        <span>{w.fieldName}</span>
        <input
          value={input.name}
          onChange={(e) => onFieldChange("name", e.target.value)}
          placeholder={w.fieldNamePlaceholder}
          required
        />
      </label>

      <div className="form__row">
        <label className="form__grow">
          <span>{w.fieldHost}</span>
          <input
            value={input.host}
            onChange={(e) => onFieldChange("host", e.target.value)}
            placeholder={w.fieldHostPlaceholder}
            required
          />
        </label>
        <label className="form__narrow">
          <span>{w.fieldPort}</span>
          <input
            type="number"
            min={1}
            max={65535}
            value={input.port}
            onChange={(e) => onFieldChange("port", Number(e.target.value))}
          />
        </label>
      </div>

      {/* The explanation sits beside the field and NOT inside the label:
          inside, it becomes part of the field's name and a screen reader
          reads the whole thing aloud on every visit. */}
      <div className="field">
        <label>
          <span>{w.fieldDomain}</span>
          <input
            value={input.domain}
            onChange={(e) => onFieldChange("domain", e.target.value)}
            placeholder="stream.example.com"
            required
          />
        </label>
        <small className="muted">{w.fieldDomainHint}</small>
      </div>

      <div className="form__row">
        <label className="form__grow">
          <span>{w.fieldUser}</span>
          <input
            value={input.user}
            onChange={(e) => onFieldChange("user", e.target.value)}
            required
          />
        </label>
        <label className="form__grow">
          <span>{w.fieldAuth}</span>
          <select
            value={input.auth_kind}
            onChange={(e) => {
              const authKind = e.target.value as ServerInput["auth_kind"];
              onFieldChange("auth_kind", authKind);
              if (authKind !== "key") onFieldChange("key_path", null);
            }}
          >
            <option value="key">{w.authKey}</option>
            <option value="password">{w.authPassword}</option>
          </select>
        </label>
      </div>

      {input.auth_kind === "key" && (
        <div className="form__inline">
          {/*
            The field stays, and stays editable: a path is as often pasted from
            somewhere as it is found by hand. The button sits **outside** the label
            on purpose — a button is a labelable element, so inside it would become
            the label's control and "Path to the private key" would point at the
            button instead of the field, for a screen reader and for every test that
            finds the field by its label.
          */}
          <label>
            <span>{w.fieldKeyPath}</span>
            <input
              value={input.key_path ?? ""}
              onChange={(e) => onFieldChange("key_path", e.target.value || null)}
              required
            />
          </label>
          <button type="button" onClick={() => void pickKey()}>
            {w.pickKey}
          </button>
        </div>
      )}

      <div className="field">
        <label>
          <span>{input.auth_kind === "key" ? w.fieldPassphrase : w.fieldPassword}</span>
          <input
            type="password"
            value={secret}
            onChange={(e) => onSecretChange(e.target.value)}
            autoComplete="off"
          />
        </label>
        <small className="muted">{secretHint}</small>
      </div>

      <details className="form__extra">
        <summary>{w.optional}</summary>
        <label>
          <span>{w.fieldVideoDir}</span>
          <input
            value={input.video_dir ?? ""}
            onChange={(e) => onFieldChange("video_dir", e.target.value || null)}
            placeholder={w.fieldVideoDirPlaceholder}
          />
        </label>
        <label>
          <span>{w.fieldCdn}</span>
          <input
            value={input.cdn_base ?? ""}
            onChange={(e) => onFieldChange("cdn_base", e.target.value || null)}
            placeholder={w.fieldCdnPlaceholder}
          />
        </label>
      </details>

      <div className="form__actions">
        <button type="button" onClick={onCancel} disabled={busy}>
          {t.ui.common.cancel}
        </button>
        <button type="submit" disabled={busy}>
          {busy ? busyLabel : submitLabel}
        </button>
      </div>
    </form>
  );
}
