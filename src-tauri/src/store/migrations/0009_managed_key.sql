-- T290a — приложение умеет само завести ключ для сервера.
--
-- **Зачем понадобился третий способ входа.** Обычный первый контакт с купленным VPS — это
-- адрес и пароль root. Развернуть с такого профиля было нельзя: шаг `ssh-key` кладёт на
-- сервер открытую половину ключа, а ключа нет, и отказ приходил внятный, но это отказ. Без
-- этого SC-014 — «от купленного VPS до работающей зрительской ссылки без единой команды в
-- консоли» — не выполняется, а именно его веха D и обещает.
--
-- **Почему не файл на диске.** Решение владельца 2026-08-27: закрытая половина лежит целиком
-- в хранилище учётных данных ОС, как и всякий другой секрет (конституция, принцип IV, без
-- оговорок). У `key` ссылка ведёт на запись с паролем от файла, а сам файл называет
-- `key_path`; у `managed_key` файла нет вовсе, и ссылка ведёт на запись с самим ключом.
--
-- Отсюда и правка ограничения: `key_path` обязан быть пустым у обоих способов, кроме `key`.
-- Ограничение SQLite не меняет, поэтому таблица делается заново и строки переносятся —
-- обычный для неё способ, и здесь безопасный: всё происходит внутри сделки самой миграции.

CREATE TABLE server_profiles_new (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    host             TEXT NOT NULL,
    port             INTEGER NOT NULL DEFAULT 22
                     CHECK (port BETWEEN 1 AND 65535),
    username         TEXT NOT NULL,
    auth_kind        TEXT NOT NULL
                     CHECK (auth_kind IN ('key', 'password', 'managed_key')),
    -- Ссылка на запись в хранилище ОС, НЕ сам секрет. У 'key' там пароль от файла ключа,
    -- у 'password' — пароль ко входу, у 'managed_key' — сам закрытый ключ.
    secret_ref       TEXT NOT NULL,
    -- Путь к файлу ключа: осмыслен только при auth_kind = 'key'. У ключа, заведённого
    -- приложением, файла нет — в этом и смысл.
    key_path         TEXT
                     CHECK ((auth_kind = 'key') OR (key_path IS NULL)),
    domain           TEXT NOT NULL,
    video_dir        TEXT NOT NULL,
    cdn_base         TEXT,
    host_fingerprint TEXT,
    ipv6_mode        TEXT
                     CHECK (ipv6_mode IS NULL OR ipv6_mode IN ('keep', 'disable')),
    is_active        INTEGER NOT NULL DEFAULT 0
                     CHECK (is_active IN (0, 1)),
    last_seen_state  TEXT,
    created_at       TEXT NOT NULL
);

INSERT INTO server_profiles_new
    (id, name, host, port, username, auth_kind, secret_ref, key_path,
     domain, video_dir, cdn_base, host_fingerprint, ipv6_mode, is_active,
     last_seen_state, created_at)
SELECT
     id, name, host, port, username, auth_kind, secret_ref, key_path,
     domain, video_dir, cdn_base, host_fingerprint, ipv6_mode, is_active,
     last_seen_state, created_at
FROM server_profiles;

DROP TABLE server_profiles;
ALTER TABLE server_profiles_new RENAME TO server_profiles;

-- «Активен ровно один» (FR-002) — правилом базы, а не аккуратностью кода. Индекс уходил
-- вместе со старой таблицей.
CREATE UNIQUE INDEX idx_server_profiles_single_active
    ON server_profiles (is_active)
    WHERE is_active = 1;
