/**
 * T053 — список серверов и карточка каждого.
 *
 * Активен ровно один (FR-002), и это видно с одного взгляда: переключатель, а не
 * галочки. Профиль без подтверждённого отпечатка помечен отдельно — он существует,
 * но подключиться по нему нельзя, и молчать об этом нельзя тем более.
 *
 * Место на диске и версия серверной части в карточке пока не показываются: первое
 * приходит вместе с библиотекой, второе появится в Фазе 7. Поле под версию
 * не рисуем впустую — пустая строка «Версия: —» ничего не сообщает.
 */

import { useEffect, useState } from "react";
import type { ServerProfile, TestStep } from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { ErrorNotice } from "../shared/ErrorNotice";
import { SetupWizard, TestSteps } from "./SetupWizard";
import { isReady, useServers } from "./store";

export function ServerList() {
  const { profiles, loading, error, reload, setActive, clearError } = useServers();
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    void reload();
  }, [reload]);

  if (loading) return <div className="panel">Читаем список серверов…</div>;

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
        <h1>Серверы</h1>
        <button onClick={() => setAdding(true)}>Добавить сервер</button>
      </div>

      {error && <ErrorNotice error={error} onDismiss={clearError} />}

      {profiles.length === 0 ? (
        <p className="muted">
          Серверов пока нет. Добавьте первый — приложение узнает его отпечаток, попросит
          вас подтвердить и проверит подключение по шагам.
        </p>
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
          <span className="server__badge">активный</span>
        ) : (
          <button onClick={onActivate}>Сделать активным</button>
        )}
      </div>

      <dl className="server__facts">
        <div>
          <dt>Домен</dt>
          <dd>{profile.domain}</dd>
        </div>
        <div>
          <dt>Каталог с видео</dt>
          <dd>{profile.video_dir}</dd>
        </div>
        {profile.cdn_base && (
          <div>
            <dt>CDN</dt>
            <dd>{profile.cdn_base}</dd>
          </div>
        )}
      </dl>

      {!isReady(profile) && (
        <p className="server__warning" role="status">
          Отпечаток сервера не подтверждён — подключиться нельзя. Приложение не отправляет
          учётные данные серверу, которого не узнаёт.
        </p>
      )}

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      {steps && <TestSteps steps={steps} />}

      <div className="server__actions">
        <button onClick={() => void runTest()} disabled={testing}>
          {testing ? "Проверяем…" : "Проверить подключение"}
        </button>
        {confirmingRemoval ? (
          <>
            <span className="server__confirm">
              Удалить профиль? Пароль или ключ от этого сервера тоже будут забыты.
            </span>
            <button className="button--danger" onClick={() => void remove()}>
              Да, удалить
            </button>
            <button onClick={() => setConfirmingRemoval(false)}>Отмена</button>
          </>
        ) : (
          <button className="button--danger" onClick={() => setConfirmingRemoval(true)}>
            Удалить
          </button>
        )}
      </div>
    </li>
  );
}
