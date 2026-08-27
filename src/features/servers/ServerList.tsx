/**
 * T053 — the list of servers and the card for each.
 *
 * Exactly one is active (FR-002), and that is visible at a glance: a switch, not
 * checkboxes. A profile with no confirmed fingerprint is marked separately — it
 * exists, but nothing can connect through it, and staying quiet about that is worse
 * still.
 *
 * Disk space is not on the card yet: it arrives with the library.
 *
 * The server-side version **is** here now (T294), and never on its own: it goes beside
 * the version this application deploys, because apart they say nothing. "Server side 1"
 * is a number until somebody says what the application deploys.
 */

import { useEffect, useState } from "react";
import type { ServerProfile, TestStep } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { useT } from "../../shared/i18n";
import { ErrorNotice } from "../shared/ErrorNotice";
import { ServerStateCard } from "./ServerStateCard";
import { SetupWizard, TestSteps } from "./SetupWizard";
import { isReady, useServers } from "./store";

export function ServerList() {
  const { profiles, loading, error, reload, setActive, clearError } = useServers();
  const [adding, setAdding] = useState(false);
  const t = useT();

  useEffect(() => {
    void reload();
  }, [reload]);

  if (loading) return <div className="panel">{t.ui.servers.reading}</div>;

  if (adding) {
    return (
      <div className="panel">
        <SetupWizard onClose={() => setAdding(false)} />
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel__head">
        <h1>{t.ui.servers.heading}</h1>
        <button onClick={() => setAdding(true)}>{t.ui.servers.add}</button>
      </div>

      {error && <ErrorNotice error={error} onDismiss={clearError} />}

      {profiles.length === 0 ? (
        <p className="muted">{t.ui.servers.empty}</p>
      ) : (
        <ul className="server-list">
          {profiles.map((p) => (
            <ServerCard
              key={p.id}
              profile={p}
              onActivate={() => void setActive(p.id)}
              onChanged={() => void reload()}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function ServerCard({
  profile,
  onActivate,
  onChanged,
}: {
  profile: ServerProfile;
  onActivate: () => void;
  onChanged: () => void;
}) {
  const [steps, setSteps] = useState<TestStep[] | null>(null);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<ReturnType<typeof toAppError> | null>(null);
  const [confirmingRemoval, setConfirmingRemoval] = useState(false);
  const t = useT();

  const runTest = async () => {
    setTesting(true);
    setError(null);
    try {
      setSteps(await ipc.serverTest(profile.id));
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setTesting(false);
    }
  };

  const remove = async () => {
    try {
      await ipc.serverRemove(profile.id);
      onChanged();
    } catch (e) {
      setError(toAppError(e));
    }
  };

  return (
    <li className={`server ${profile.is_active ? "server--active" : ""}`}>
      <div className="server__head">
        <div>
          <span className="server__name">{profile.name}</span>
          <span className="server__addr">
            {profile.user}@{profile.host}
            {profile.port !== 22 && `:${profile.port}`}
          </span>
        </div>
        {profile.is_active ? (
          <span className="server__badge">{t.ui.servers.activeBadge}</span>
        ) : (
          <button onClick={onActivate}>{t.ui.servers.makeActive}</button>
        )}
      </div>

      {/* Что это за сервер — сразу под именем, до подробностей профиля. Человек, открывший
          список, чаще всего пришёл именно за этим: развёрнут ли он, чей он, не пора ли его
          обновить (T294). */}
      {profile.host_fingerprint && <ServerStateCard serverId={profile.id} />}

      <dl className="server__facts">
        <div>
          <dt>{t.ui.servers.domain}</dt>
          <dd>{profile.domain}</dd>
        </div>
        <div>
          <dt>{t.ui.servers.videoDir}</dt>
          <dd>{profile.video_dir}</dd>
        </div>
        {profile.cdn_base && (
          <div>
            <dt>{t.ui.servers.cdn}</dt>
            <dd>{profile.cdn_base}</dd>
          </div>
        )}
      </dl>

      {!isReady(profile) && (
        <p className="server__warning" role="status">
          {t.ui.servers.fingerprintUnconfirmed}
        </p>
      )}

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      {steps && <TestSteps steps={steps} />}

      <div className="server__actions">
        <button onClick={() => void runTest()} disabled={testing}>
          {testing ? t.ui.servers.testing : t.ui.servers.test}
        </button>
        {confirmingRemoval ? (
          <>
            <span className="server__confirm">{t.ui.servers.confirmRemoval}</span>
            <button className="button--danger" onClick={() => void remove()}>
              {t.ui.servers.removeYes}
            </button>
            <button onClick={() => setConfirmingRemoval(false)}>
              {t.ui.common.cancel}
            </button>
          </>
        ) : (
          <button className="button--danger" onClick={() => setConfirmingRemoval(true)}>
            {t.ui.servers.remove}
          </button>
        )}
      </div>
    </li>
  );
}
