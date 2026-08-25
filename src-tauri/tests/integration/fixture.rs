//! T026 — одноразовый сервер для интеграционных тестов.
//!
//! Контейнер поднимается тестом и удаляется вместе с ним. Боевой сервер не используется
//! **никогда** (конституция, раздел «Порядок работы»): проверки здесь меняют состояние
//! сервера, а на боевом это оборвало бы просмотр.
//!
//! Что контейнер закрывает и чего не закрывает — важно понимать, чтобы не считать
//! проверку полнее, чем она есть:
//!
//! | | контейнер | настоящий VPS |
//! |---|---|---|
//! | файловые операции, команды, опись, передача с обрывами | да | да |
//! | службы, сетевой фильтр, настройка ядра, сертификат | **нет** | да |
//!
//! Поэтому Фаза 7 проверяется на выделенном тестовом VPS, а не здесь.

use std::process::Command;
use std::time::{Duration, Instant};

/// Имя образа. Собирается один раз и переиспользуется между запусками тестов.
const IMAGE: &str = "vrcast-test-sshd:1";

/// Парольная фраза ключа из `tests/fixtures`. Не секрет: ключ никуда не даёт доступа,
/// кроме этого одноразового контейнера.
pub const KEY_PASSPHRASE: &str = "тестовая-фраза-1234";

/// Пароль внутри контейнера — для проверки второго способа входа.
pub const ROOT_PASSWORD: &str = "тестовый-пароль-контейнера";

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn key_path() -> std::path::PathBuf {
    fixtures_dir().join("encrypted_ed25519.key")
}

fn docker(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("docker").args(args).output()
}

/// Есть ли работающий Docker.
///
/// Без него интеграционные тесты не запускаются — но и не притворяются пройденными:
/// тест, который молча пропускается, хуже отсутствующего, потому что создаёт видимость
/// проверки.
pub fn docker_available() -> bool {
    matches!(docker(&["info", "--format", "{{.ServerVersion}}"]), Ok(o) if o.status.success())
}

fn ensure_image() -> Result<(), String> {
    let exists = docker(&["image", "inspect", IMAGE])
        .map(|o| o.status.success())
        .unwrap_or(false);
    if exists {
        return Ok(());
    }

    let dir = fixtures_dir().join("docker");
    let out = docker(&[
        "build",
        "-t",
        IMAGE,
        "-f",
        dir.join("Dockerfile").to_string_lossy().as_ref(),
        dir.to_string_lossy().as_ref(),
    ])
    .map_err(|e| format!("не запустить сборку образа: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "образ не собрался:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Работающий одноразовый сервер.
///
/// Удаляется при уничтожении значения — в том числе когда тест падает: иначе после
/// неудачного прогона в системе оставались бы висящие контейнеры.
pub struct TestServer {
    id: String,
    pub port: u16,
}

impl TestServer {
    /// Поднять контейнер и дождаться, пока он начнёт принимать соединения.
    pub fn start() -> Result<Self, String> {
        // Первым делом — понятная причина вместо невнятного «контейнер не поднялся».
        // Поймано на живом прогоне 2026-08-25: Docker ушёл в спячку, и десять тестов
        // упали с сообщением, из которого причина не читалась.
        if !docker_available() {
            return Err(String::from(
                "Docker не запущен. Откройте Docker Desktop и повторите: интеграционные тесты поднимают одноразовый сервер в контейнере.",
            ));
        }
        ensure_image()?;

        // Порт выбирает система: тесты могут идти одновременно с чем угодно,
        // и занимать заранее известный порт нельзя.
        let out = docker(&["run", "-d", "--rm", "-p", "127.0.0.1::22", IMAGE])
            .map_err(|e| format!("не запустить контейнер: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "контейнер не запустился:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();

        // Значение владеет контейнером с этого мгновения: если разбор порта или ожидание
        // готовности не удадутся, контейнер уберётся при уничтожении, а не останется висеть.
        let mut server = Self { id, port: 0 };
        server.port = server.discover_port()?;
        server.wait_until_ready()?;
        Ok(server)
    }

    fn discover_port(&self) -> Result<u16, String> {
        let out =
            docker(&["port", &self.id, "22/tcp"]).map_err(|e| format!("не узнать порт: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .find_map(|l| l.rsplit(':').next())
            .and_then(|p| p.trim().parse::<u16>().ok())
            .ok_or_else(|| format!("порт не разобрать из «{}»", text.trim()))
    }

    /// Дождаться, пока сервер начнёт принимать соединения.
    ///
    /// Служба поднимается не мгновенно, и подключение сразу после запуска контейнера
    /// упало бы не из-за нашего кода. Ждём именно готовности, а не «просто подождём».
    fn wait_until_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect_timeout(
                &format!("127.0.0.1:{}", self.port)
                    .parse()
                    .map_err(|e| format!("неверный адрес: {e}"))?,
                Duration::from_millis(500),
            )
            .is_ok()
            {
                // Соединение принимается, но службе нужен ещё миг на готовность к обмену.
                std::thread::sleep(Duration::from_millis(300));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(String::from(
            "сервер в контейнере не начал принимать соединения за 30 секунд",
        ))
    }

    /// Выполнить команду внутри контейнера, минуя наш слой доступа.
    ///
    /// Нужно, чтобы готовить условия и проверять последствия **независимо** от того кода,
    /// который мы и проверяем: сверять результат тем же средством, которым его получили, —
    /// значит не проверять ничего.
    pub fn exec_inside(&self, script: &str) -> Result<String, String> {
        let out = docker(&["exec", &self.id, "bash", "-lc", script])
            .map_err(|e| format!("не выполнить команду в контейнере: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "команда в контейнере не удалась: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn host(&self) -> &'static str {
        "127.0.0.1"
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Контейнер запущен с самоудалением, но останавливаем явно: иначе после
        // упавшего теста он остался бы висеть до конца сеанса.
        let _ = docker(&["kill", &self.id]);
    }
}
