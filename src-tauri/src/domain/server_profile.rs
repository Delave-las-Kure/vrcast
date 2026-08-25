//! T029 — профиль сервера и его проверки (`data-model.md` §1).
//!
//! Профиль **не содержит секрета** — только ссылку на запись в хранилище ОС
//! (конституция, принцип IV). Это не соглашение, а свойство типа: поля под пароль
//! здесь просто нет, и положить его в профиль некуда.

use super::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// Каталог раздачи по умолчанию.
///
/// Это единственное место во всём приложении, где путь раздачи написан буквами.
/// Он подставляется в новый профиль, тут же доступен пользователю для правки, и
/// дальше приложение берёт путь **только** из профиля (FR-004). Пометка в конце
/// строки — то, по чему `scripts/check-no-hardcoded-server.sh` отличает это
/// умолчание от случайно занесённой привязки к чужому серверу.
pub const DEFAULT_VIDEO_DIR: &str = "/var/lib/vrcast/videos"; // FR-004-ok: значение по умолчанию

/// Порт SSH по умолчанию.
pub const DEFAULT_SSH_PORT: u16 = 22;

/// Предел длины имени профиля — чтобы список оставался читаемым.
const MAX_NAME_LEN: usize = 100;

/// Способ входа на сервер.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Key,
    Password,
}

impl AuthKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Password => "password",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "key" => Some(Self::Key),
            "password" => Some(Self::Password),
            _ => None,
        }
    }
}

/// Что делать с IPv6 при развёртывании (FR-135). `None` в профиле = пользователь
/// ещё не выбирал; молчаливое умолчание здесь недопустимо.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Mode {
    Keep,
    Disable,
}

impl Ipv6Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Disable => "disable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "keep" => Some(Self::Keep),
            "disable" => Some(Self::Disable),
            _ => None,
        }
    }
}

/// Профиль сервера. Хранится в локальной базе.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_kind: AuthKind,
    /// Ссылка на запись в хранилище ОС — **не сам секрет**.
    pub secret_ref: String,
    /// Путь к файлу ключа. Только при `auth_kind = Key`.
    pub key_path: Option<String>,
    /// Домен раздачи. Обязателен: без него нельзя ни выдать ссылку, ни проверить
    /// работоспособность раздачи (FR-125).
    pub domain: String,
    pub video_dir: String,
    /// Пусто = ссылки только с origin (FR-016).
    pub cdn_base: Option<String>,
    pub host_fingerprint: Option<String>,
    pub ipv6_mode: Option<Ipv6Mode>,
    pub is_active: bool,
}

/// Что именно не так с профилем.
///
/// Проверка возвращает **все** замечания сразу, а не первое: в мастере настройки
/// пользователь заполняет форму целиком, и показывать ошибки по одной — значит
/// заставлять его проходить круг заново из-за каждой опечатки.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileProblem {
    /// Which field to highlight in the form.
    pub field: &'static str,
    /// What to say about it. The wording is the interface's (FR-105, FR-106).
    pub detail: Detail,
}

impl ProfileProblem {
    fn new(field: &'static str, key: DetailCode) -> Self {
        Self {
            field,
            detail: Detail::new(key),
        }
    }

    /// An objection that names a number: a length limit, an allowed range.
    fn with(field: &'static str, detail: Detail) -> Self {
        Self { field, detail }
    }
}

impl std::fmt::Display for ProfileProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.detail.key)
    }
}

impl ServerProfile {
    /// Новый профиль с разумными умолчаниями. Проверку всё равно надо пройти.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            host: String::new(),
            port: DEFAULT_SSH_PORT,
            user: String::from("root"),
            auth_kind: AuthKind::Key,
            secret_ref: String::new(),
            key_path: None,
            domain: String::new(),
            video_dir: String::from(DEFAULT_VIDEO_DIR),
            cdn_base: None,
            host_fingerprint: None,
            ipv6_mode: None,
            is_active: false,
        }
    }

    /// Привести поля к каноническому виду: убрать пробелы по краям, снять схему и
    /// хвостовую косую черту с домена, хвостовую косую с путей.
    ///
    /// Приведение отделено от проверки намеренно. Люди вставляют домен из адресной
    /// строки браузера — вместе с `https://` и косой чертой. Отвергать за это значит
    /// придираться: намерение однозначно. А вот путь с `..` уже не приведёшь — про
    /// него скажет проверка.
    pub fn normalize(&mut self) {
        self.id = self.id.trim().to_owned();
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.user = self.user.trim().to_owned();
        self.secret_ref = self.secret_ref.trim().to_owned();
        self.domain = normalize_domain(&self.domain);
        self.video_dir = normalize_dir(&self.video_dir);

        self.key_path = self
            .key_path
            .take()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());
        self.cdn_base = self
            .cdn_base
            .take()
            .map(|b| b.trim().trim_end_matches('/').to_owned())
            .filter(|b| !b.is_empty());
        self.host_fingerprint = self
            .host_fingerprint
            .take()
            .map(|f| f.trim().to_owned())
            .filter(|f| !f.is_empty());

        // Ключ имеет смысл только при входе по ключу. Оставлять его при входе по
        // паролю — значит хранить путь, который однажды применят не к тому профилю.
        if self.auth_kind == AuthKind::Password {
            self.key_path = None;
        }
    }

    /// Проверить профиль целиком. Перед проверкой стоит вызвать [`Self::normalize`].
    pub fn validate(&self) -> Result<(), Vec<ProfileProblem>> {
        let mut problems = Vec::new();

        if self.id.trim().is_empty() {
            problems.push(ProfileProblem::new("id", DetailCode::ProfileIdEmpty));
        }

        if self.name.trim().is_empty() {
            problems.push(ProfileProblem::new("name", DetailCode::ProfileNameEmpty));
        } else if self.name.chars().count() > MAX_NAME_LEN {
            problems.push(ProfileProblem::with(
                "name",
                Detail::new(DetailCode::ProfileNameTooLong).with("max", MAX_NAME_LEN),
            ));
        }

        if self.host.is_empty() {
            problems.push(ProfileProblem::new("host", DetailCode::ProfileHostEmpty));
        } else if self.host.contains(char::is_whitespace) || self.host.contains('/') {
            problems.push(ProfileProblem::new("host", DetailCode::ProfileHostNotBare));
        }

        if self.port == 0 {
            problems.push(ProfileProblem::new("port", DetailCode::ProfilePortRange));
        }

        if self.user.is_empty() {
            problems.push(ProfileProblem::new("user", DetailCode::ProfileUserEmpty));
        } else if self.user.contains(char::is_whitespace) {
            problems.push(ProfileProblem::new(
                "user",
                DetailCode::ProfileUserHasSpaces,
            ));
        }

        if self.secret_ref.is_empty() {
            problems.push(ProfileProblem::new(
                "secret_ref",
                DetailCode::ProfileSecretRefEmpty,
            ));
        }

        match self.auth_kind {
            AuthKind::Key => {
                if self.key_path.as_deref().unwrap_or("").is_empty() {
                    problems.push(ProfileProblem::new(
                        "key_path",
                        DetailCode::ProfileKeyPathRequired,
                    ));
                }
            }
            AuthKind::Password => {
                if self.key_path.is_some() {
                    problems.push(ProfileProblem::new(
                        "key_path",
                        DetailCode::ProfileKeyPathUnused,
                    ));
                }
            }
        }

        if let Err(key) = check_domain(&self.domain) {
            problems.push(ProfileProblem::new("domain", key));
        }

        if let Err(key) = check_dir(&self.video_dir) {
            problems.push(ProfileProblem::new("video_dir", key));
        }

        if let Some(base) = &self.cdn_base {
            if let Err(key) = check_cdn_base(base) {
                problems.push(ProfileProblem::new("cdn_base", key));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// Привести домен к каноническому виду: без схемы, без хвостовой косой, в нижнем регистре.
///
/// Регистр снимается ПЕРВЫМ делом. Иначе вставленное из адресной строки `HTTPS://…`
/// не совпадёт с образцом схемы, и она останется внутри домена — а дальше ссылка
/// соберётся с удвоенной схемой и молча перестанет работать.
pub fn normalize_domain(raw: &str) -> String {
    let lowered = raw.trim().to_lowercase();
    let mut d = lowered.as_str();
    for prefix in ["https://", "http://"] {
        if let Some(rest) = d.strip_prefix(prefix) {
            d = rest;
            break;
        }
    }
    d.trim_end_matches('/').to_owned()
}

/// Привести путь к каталогу к каноническому виду: без хвостовой косой черты.
fn normalize_dir(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() > 1 {
        trimmed.trim_end_matches('/').to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn check_domain(domain: &str) -> Result<(), DetailCode> {
    if domain.is_empty() {
        return Err(DetailCode::DomainEmpty);
    }
    if domain.contains(char::is_whitespace) {
        return Err(DetailCode::DomainHasSpaces);
    }
    if domain.contains('/') {
        return Err(DetailCode::DomainHasPath);
    }
    if domain.contains('@') || domain.contains(':') {
        return Err(DetailCode::DomainHasUserOrPort);
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return Err(DetailCode::DomainBadDots);
    }
    if !domain.contains('.') {
        return Err(DetailCode::DomainNoDot);
    }
    if domain
        .chars()
        .any(|c| !(c.is_alphanumeric() || c == '-' || c == '.'))
    {
        return Err(DetailCode::DomainBadChars);
    }
    Ok(())
}

fn check_dir(dir: &str) -> Result<(), DetailCode> {
    if dir.is_empty() {
        return Err(DetailCode::VideoDirEmpty);
    }
    if !dir.starts_with('/') {
        return Err(DetailCode::VideoDirNotAbsolute);
    }
    // Отрезки `..` опасны не теоретически: путь отсюда попадает в команды на сервере,
    // и один такой отрезок выводит запись за пределы каталога раздачи.
    if dir.split('/').any(|part| part == "..") {
        return Err(DetailCode::VideoDirHasDotDot);
    }
    if dir.contains('\n') || dir.contains('\r') {
        return Err(DetailCode::VideoDirHasNewline);
    }
    Ok(())
}

fn check_cdn_base(base: &str) -> Result<(), DetailCode> {
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(DetailCode::CdnBaseNoScheme);
    }
    if base.contains(char::is_whitespace) {
        return Err(DetailCode::CdnBaseHasSpaces);
    }
    let rest = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or("");
    if rest.is_empty() {
        return Err(DetailCode::CdnBaseIncomplete);
    }
    Ok(())
}
