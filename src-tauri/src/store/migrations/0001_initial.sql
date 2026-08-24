-- Миграция 0001 — начальная схема.
-- Уже выпущенную миграцию менять нельзя: только добавлять следующую.
--
-- Здесь нет ни одного поля под секрет. Пароли, парольные фразы и приватные ключи
-- лежат в хранилище учётных данных ОС; сюда попадает только ссылка на запись
-- (конституция, принцип IV; FR-090).

CREATE TABLE server_profiles (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    host             TEXT NOT NULL,
    port             INTEGER NOT NULL DEFAULT 22
                     CHECK (port BETWEEN 1 AND 65535),
    username         TEXT NOT NULL,
    auth_kind        TEXT NOT NULL
                     CHECK (auth_kind IN ('key', 'password')),
    -- Ссылка на запись в хранилище ОС, НЕ сам секрет.
    secret_ref       TEXT NOT NULL,
    -- Путь к файлу ключа: осмыслен только при auth_kind = 'key'.
    key_path         TEXT
                     CHECK ((auth_kind = 'key') OR (key_path IS NULL)),
    -- Домен раздачи. Обязателен: без него нельзя ни выдать зрительскую ссылку,
    -- ни проверить работоспособность раздачи (FR-125).
    domain           TEXT NOT NULL,
    video_dir        TEXT NOT NULL,
    -- Пусто = раздаём только с origin (FR-016).
    cdn_base         TEXT,
    -- Запоминается при первом подключении; расхождение = предупреждение (FR-092).
    host_fingerprint TEXT,
    -- Выбор пользователя при развёртывании (FR-135). NULL = ещё не выбирал.
    ipv6_mode        TEXT
                     CHECK (ipv6_mode IS NULL OR ipv6_mode IN ('keep', 'disable')),
    is_active        INTEGER NOT NULL DEFAULT 0
                     CHECK (is_active IN (0, 1)),
    -- Последнее известное состояние сервера, чтобы показать его с пометкой
    -- «данные устарели», когда сервер недоступен.
    last_seen_state  TEXT,
    created_at       TEXT NOT NULL
);

-- «Активен ровно один» (FR-002) — правилом базы, а не аккуратностью кода.
-- Частичный индекс допускает много нулей и не более одной единицы.
CREATE UNIQUE INDEX idx_server_profiles_single_active
    ON server_profiles (is_active)
    WHERE is_active = 1;

CREATE TABLE tasks (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL
                  CHECK (kind IN ('probe', 'convert', 'upload', 'build_ladder',
                                  'deploy', 'upgrade_server', 'diagnose')),
    -- NULL для чисто локальных задач (разбор исходника, подготовка файла).
    server_id     TEXT REFERENCES server_profiles (id) ON DELETE CASCADE,
    state         TEXT NOT NULL
                  CHECK (state IN ('queued', 'running', 'paused',
                                   'completed', 'failed', 'cancelled')),
    progress      REAL NOT NULL DEFAULT 0
                  CHECK (progress BETWEEN 0 AND 1),
    stage         TEXT,
    speed_bps     INTEGER,
    eta_s         INTEGER,
    -- Позиция возобновления: переданные байты, готовые ступени, выполненные шаги.
    resume_token  TEXT,
    -- Человеческая формулировка (FR-105), уже прошедшая вырезание секретов.
    error         TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX idx_tasks_state ON tasks (state);
CREATE INDEX idx_tasks_server ON tasks (server_id);

-- Отпечатки серверов, встреченные ранее. Хранится отдельно от профиля: отпечаток
-- принадлежит паре «адрес + порт», а профилей на один адрес может быть несколько.
CREATE TABLE host_fingerprints (
    host        TEXT NOT NULL,
    port        INTEGER NOT NULL,
    fingerprint TEXT NOT NULL,
    first_seen  TEXT NOT NULL,
    PRIMARY KEY (host, port)
);
