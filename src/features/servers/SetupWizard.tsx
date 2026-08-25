/**
 * T052 — мастер настройки сервера.
 *
 * Три шага, и порядок между ними обязателен:
 *
 * 1. **Данные.** Форма с полями профиля. Ошибки показываются все сразу — человек
 *    заполняет её целиком, и гонять его по кругу из-за каждой опечатки незачем.
 * 2. **Отпечаток.** Приложение узнаёт отпечаток сервера, **ничего ему не предъявляя**,
 *    и показывает человеку. Пока он не подтверждён, ни пароль, ни ключ на сервер
 *    не уходят (FR-092). Это единственный шаг, который нельзя пропустить.
 * 3. **Проверка.** Четыре шага подряд: сеть, вход, каталог с видео, отдача по домену.
 *    Показываются все — с отметкой, где остановилось (FR-003).
 *
 * Если рядом нашёлся `server.env` от прежнего порядка работы, поля предлагается
 * заполнить из него (T043). Файл при этом только читается.
 */

import { useEffect, useState } from "react";
import type {
  AppError,
  ImportSuggestion,
  ServerInput,
  TestStep,
} from "../../shared/contract";
import { ipc, toAppError } from "../../shared/ipc";
import { ErrorNotice } from "../shared/ErrorNotice";
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

  // Предложение перенести настройки ищется один раз при открытии. Его отсутствие —
  // обычное дело, а не беда: у большинства пользователей файла нет и не будет.
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

  const field = <K extends keyof ServerInput>(key: K, value: ServerInput[K]) =>
    setInput((prev) => ({ ...prev, [key]: value }));

  /** Шаг 1 → 2: создать профиль и узнать отпечаток. */
  const submitForm = async () => {
    setBusy(true);
    setError(null);
    try {
      const id = await ipc.serverAdd(input, secret);
      setServerId(id);
      // Секрет из памяти интерфейса больше не нужен: он ушёл в хранилище системы
      // и обратно не возвращается никогда.
      setSecret("");
      setFingerprint(await ipc.serverProbeFingerprint(input.host, input.port));
      setStage("fingerprint");
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  /** Шаг 2 → 3: подтвердить отпечаток и прогнать проверку. */
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

  /** Отказ на шаге отпечатка: профиль уже создан, и оставлять его нельзя. */
  const abandon = async () => {
    if (serverId) {
      try {
        await ipc.serverRemove(serverId);
      } catch {
        // Уборка не удалась — покажется в списке, оттуда его можно удалить руками.
        // Прерывать закрытие из-за этого нельзя: человек уже решил отказаться.
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
    <div className="wizard" role="dialog" aria-label="Настройка сервера">
      <header className="wizard__head">
        <h2>Новый сервер</h2>
        <ol className="wizard__steps">
          <li className={stage === "form" ? "is-current" : "is-done"}>Данные</li>
          <li
            className={
              stage === "fingerprint" ? "is-current" : stage === "form" ? "" : "is-done"
            }
          >
            Отпечаток
          </li>
          <li className={stage === "test" || stage === "done" ? "is-current" : ""}>
            Проверка
          </li>
        </ol>
      </header>

      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      {stage === "form" && (
        <>
          {suggestion && (
            <div className="notice" role="status">
              <div className="notice__body">
                <strong className="notice__message">
                  Рядом нашлись настройки от прежнего способа работы
                </strong>
                <p className="notice__hint">
                  {suggestion.source} — можно подставить адрес, домен, пользователя и путь
                  к ключу. Файл только читается и не изменяется.
                  {suggestion.needs_passphrase &&
                    " Парольную фразу ключа придётся ввести: в файле её нет."}
                </p>
              </div>
              <button
                onClick={() => {
                  setInput(suggestion.input);
                  setSuggestion(null);
                }}
              >
                Подставить
              </button>
            </div>
          )}

          <form
            className="form"
            onSubmit={(e) => {
              e.preventDefault();
              void submitForm();
            }}
          >
            <label>
              <span>Название</span>
              <input
                value={input.name}
                onChange={(e) => field("name", e.target.value)}
                placeholder="Как отличать этот сервер от других"
                required
              />
            </label>

            <div className="form__row">
              <label className="form__grow">
                <span>Адрес</span>
                <input
                  value={input.host}
                  onChange={(e) => field("host", e.target.value)}
                  placeholder="IP-адрес или имя"
                  required
                />
              </label>
              <label className="form__narrow">
                <span>Порт</span>
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={input.port}
                  onChange={(e) => field("port", Number(e.target.value))}
                />
              </label>
            </div>

            {/* Пояснение стоит рядом с полем, а НЕ внутри метки: иначе оно попадает
                в название поля, и озвучиватель экрана читает его целиком вслух
                при каждом переходе. */}
            <div className="field">
              <label>
                <span>Домен раздачи</span>
                <input
                  value={input.domain}
                  onChange={(e) => field("domain", e.target.value)}
                  placeholder="stream.example.com"
                  required
                />
              </label>
              <small className="muted">
                По нему выдаются зрительские ссылки. Можно вставить прямо из адресной
                строки — лишнее уберётся само.
              </small>
            </div>

            <div className="form__row">
              <label className="form__grow">
                <span>Пользователь</span>
                <input
                  value={input.user}
                  onChange={(e) => field("user", e.target.value)}
                  required
                />
              </label>
              <label className="form__grow">
                <span>Вход</span>
                <select
                  value={input.auth_kind}
                  onChange={(e) =>
                    setInput((prev) => ({
                      ...prev,
                      auth_kind: e.target.value as ServerInput["auth_kind"],
                      key_path: e.target.value === "key" ? prev.key_path : null,
                    }))
                  }
                >
                  <option value="key">По ключу</option>
                  <option value="password">По паролю</option>
                </select>
              </label>
            </div>

            {input.auth_kind === "key" && (
              <label>
                <span>Путь к приватному ключу</span>
                <input
                  value={input.key_path ?? ""}
                  onChange={(e) => field("key_path", e.target.value || null)}
                  placeholder="C:\Users\...\.ssh\id_ed25519"
                  required
                />
              </label>
            )}

            <div className="field">
              <label>
                <span>
                  {input.auth_kind === "key" ? "Парольная фраза ключа" : "Пароль"}
                </span>
                <input
                  type="password"
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                  autoComplete="off"
                />
              </label>
              <small className="muted">
                Сохраняется в хранилище паролей системы, а не в файлах приложения.
                Обратно приложению он не возвращается.
              </small>
            </div>

            <details className="form__extra">
              <summary>Необязательное</summary>
              <label>
                <span>Каталог с видео на сервере</span>
                <input
                  value={input.video_dir ?? ""}
                  onChange={(e) => field("video_dir", e.target.value || null)}
                  placeholder="оставьте пустым для значения по умолчанию"
                />
              </label>
              <label>
                <span>Адрес CDN</span>
                <input
                  value={input.cdn_base ?? ""}
                  onChange={(e) => field("cdn_base", e.target.value || null)}
                  placeholder="пусто = ссылки только через сам сервер"
                />
              </label>
            </details>

            <div className="form__actions">
              <button type="button" onClick={onClose} disabled={busy}>
                Отмена
              </button>
              <button type="submit" disabled={busy}>
                {busy ? "Проверяем…" : "Дальше"}
              </button>
            </div>
          </form>
        </>
      )}

      {stage === "fingerprint" && fingerprint && (
        <section className="wizard__stage">
          <p>
            Сервер представился вот этим отпечатком. Сверьте его с тем, что показывает
            панель управления вашего хостера, — и подтвердите.
          </p>
          <code className="fingerprint">{fingerprint}</code>
          <p className="muted">
            До подтверждения приложение не отправит серверу ни пароль, ни ключ. Так
            подменённый сервер не получит ваши учётные данные, даже если сумеет
            притвориться нужным адресом.
          </p>
          <div className="form__actions">
            <button type="button" onClick={() => void abandon()} disabled={busy}>
              Отказаться
            </button>
            <button type="button" onClick={() => void confirmFingerprint()} disabled={busy}>
              {busy ? "Проверяем…" : "Отпечаток верный"}
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
              Проверить снова
            </button>
            <button type="button" onClick={() => void finish()} disabled={busy}>
              Готово
            </button>
          </div>
        </section>
      )}
    </div>
  );
}

/**
 * Показ шагов проверки.
 *
 * Показываются **все** шаги, включая невыполнявшиеся: человеку нужно видеть,
 * что успело пройти, а не только последнюю беду (FR-003).
 */
export function TestSteps({ steps }: { steps: TestStep[] | null }) {
  if (!steps) return <p className="muted">Проверяем подключение…</p>;

  return (
    <ol className="steps">
      {steps.map((s) => (
        <li key={s.id} className={`step step--${s.status}`}>
          <span className="step__mark" aria-hidden="true">
            {s.status === "ok" ? "✓" : s.status === "failed" ? "✕" : "·"}
          </span>
          <div className="step__body">
            <span className="step__title">{s.title}</span>
            {s.detail && <span className="step__detail">{s.detail}</span>}
            {s.status === "skipped" && !s.detail && (
              <span className="step__detail muted">не проверяли: остановились раньше</span>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
