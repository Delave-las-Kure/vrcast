/**
 * T356–T358 — «убрать мои данные» изнутри приложения (FR-114).
 *
 * **Почему это живёт здесь, а не только в деинсталляторе.** Из трёх форматов поставки спросить
 * при удалении может ровно один: у деинсталлятора Windows есть флажок, `deb` выполняет свой
 * сценарий удаления без всякого диалога, а AppImage не устанавливается вовсе — это файл,
 * который стёрли. Приложение — единственное место, которое есть у всех троих.
 *
 * **И только оно достаёт секреты.** Они лежат в хранилище операционной системы, а не в
 * каталоге данных: ни флажок, ни `postrm` их не трогают. После удаления убирать их некому.
 *
 * **Список, а не обещание.** «Удалить мои данные» без перечня каждый читает по-своему, а
 * решает тот, кто потом уже не проверит.
 */

import { useCallback, useEffect, useState } from "react";

import { ErrorNotice } from "../shared/ErrorNotice";
import { useLang, useT } from "../../shared/i18n";
import { formatBytes } from "../../shared/i18n/format";
import { ipc } from "../../shared/ipc";
import type { AppError, WhatWent, WhatWouldGo } from "../../shared/contract";

export function Forget() {
  const t = useT();
  const { lang } = useLang();
  const words = t.ui.forget;

  const [would, setWould] = useState<WhatWouldGo | null>(null);
  const [went, setWent] = useState<WhatWent | null>(null);
  const [agreed, setAgreed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let alive = true;
    ipc
      .forgetPreview()
      .then((got) => {
        if (alive) setWould(got);
      })
      .catch((e: AppError) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, []);

  const remove = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setWent(await ipc.forgetEverything(true));
    } catch (e) {
      setError(e as AppError);
    } finally {
      setBusy(false);
    }
  }, []);

  if (went) {
    return (
      <section className="forget">
        <h3>{words.title}</h3>
        <p data-testid="forget-done">{words.done}</p>
        {went.secrets_left.length > 0 && (
          // Сказано вслух: человек, которому ответили «всё убрано», пока записи остались,
          // услышал неправду и проверить её ему нечем.
          <p className="forget-warning" data-testid="forget-left">
            {words.secretsLeft(went.secrets_left.join(", "))}
          </p>
        )}
        {!went.data_dir_removed && (
          <p className="forget-warning" data-testid="forget-dir-left">
            {words.dirLeft}
          </p>
        )}
      </section>
    );
  }

  return (
    <section className="forget">
      <h3>{words.title}</h3>
      <p className="appearance__means">{words.means}</p>
      {error && <ErrorNotice error={error} />}

      {would && (
        <>
          <ul className="forget-list" data-testid="forget-list">
            {would.data_dir && (
              <li>
                {words.dataDir}: <code>{would.data_dir}</code> ({formatBytes(would.bytes, lang)})
              </li>
            )}
            <li data-testid="forget-servers">
              {words.servers}: {would.servers.length > 0 ? would.servers.join(", ") : words.none}
            </li>
            <li>
              {words.secrets}: {would.secrets}
            </li>
          </ul>

          {would.locked_out.length > 0 && (
            // **Единственная потеря, которую нельзя отменить.** У сервера, развёрнутого этим
            // приложением, вход по паролю выключен, и ключ к нему есть только здесь.
            <div className="forget-danger" data-testid="forget-locked-out">
              <p>{words.lockedOut(would.locked_out.join(", "))}</p>
              <p>{words.lockedOutAdvice}</p>
            </div>
          )}

          <label className="forget-agree">
            <input
              type="checkbox"
              checked={agreed}
              onChange={(e) => setAgreed(e.target.checked)}
              data-testid="forget-agree"
            />
            {words.agree}
          </label>

          <button
            type="button"
            className="danger"
            disabled={!agreed || busy}
            onClick={() => void remove()}
            data-testid="forget-do"
          >
            {busy ? words.removing : words.remove}
          </button>
        </>
      )}
    </section>
  );
}
