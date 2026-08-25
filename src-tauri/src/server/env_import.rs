//! T043 — разовый перенос настроек из `server.env`.
//!
//! Файл `server.env` — то, чем автор пользовался до появления приложения, и он
//! продолжает работать: скиллы читают его как читали (конституция, принцип VII).
//! Приложение предлагает **перенести** оттуда параметры в первый профиль, чтобы
//! не заставлять набирать заново то, что уже записано.
//!
//! Три правила, и все три существенны:
//!
//! 1. **Только чтение.** Файл не изменяется и не переписывается ни при каких
//!    условиях: он принадлежит прежнему порядку работы, а не приложению.
//! 2. **Разово.** После создания профиля приложение к файлу не возвращается.
//!    Иначе получилось бы два источника правды, и правка в приложении молча
//!    расходилась бы с файлом.
//! 3. **Пароль не переносится.** В файле он обычно пуст, а если не пуст — это
//!    запасной вход через консоль хостера, а не то, чем стоит пользоваться
//!    приложению. Парольную фразу ключа человек вводит сам: в файле её нет.

use crate::commands::servers::ServerInput;
use crate::domain::server_profile::{AuthKind, DEFAULT_VIDEO_DIR};
use std::path::{Path, PathBuf};

/// Что удалось вычитать из `server.env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Готовые поля профиля. Пользователь видит их в мастере и может поправить.
    pub input: ServerInput,
    /// Откуда взято — показывается человеку, чтобы он понимал, что происходит.
    pub source: PathBuf,
    /// Нужна ли парольная фраза: ключ есть, а фразы в файле нет и быть не может.
    pub needs_passphrase: bool,
}

/// Где искать `server.env` относительно каталога приложения.
///
/// Приложение лежит в `vrcast-studio/`, файл — рядом с ним, в корне рабочего
/// каталога прежнего порядка работы.
pub fn default_location() -> Option<PathBuf> {
    let exe = std::env::current_dir().ok()?;
    for dir in exe.ancestors().take(4) {
        let candidate = dir.join("server.env");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Прочитать `server.env` и собрать поля профиля.
///
/// Возвращает `None`, если файла нет или в нём не нашлось главного — адреса
/// и домена. Отсутствие файла не ошибка: у большинства пользователей приложения
/// его и не будет.
pub fn read_from(path: &Path) -> Option<Imported> {
    let text = std::fs::read_to_string(path).ok()?;
    let values = parse(&text);

    let host = values.get("SERVER_IP").cloned().unwrap_or_default();
    let domain = values.get("SERVER_DOMAIN").cloned().unwrap_or_default();
    if host.is_empty() || domain.is_empty() {
        return None;
    }

    let key_path = values
        .get("SSH_KEY")
        .map(|k| expand_home(k))
        .filter(|k| !k.is_empty());

    Some(Imported {
        input: ServerInput {
            name: domain.clone(),
            host,
            port: 22,
            user: values
                .get("SSH_USER")
                .cloned()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| String::from("root")),
            // Вход по ключу, даже если в файле указан и пароль: пароль там —
            // запасной вход через консоль хостера, а не рабочий способ.
            auth_kind: if key_path.is_some() {
                AuthKind::Key
            } else {
                AuthKind::Password
            },
            key_path: key_path.clone(),
            domain,
            video_dir: values
                .get("VIDEO_DIR")
                .cloned()
                .filter(|d| !d.is_empty())
                .or_else(|| Some(String::from(DEFAULT_VIDEO_DIR))),
            cdn_base: values.get("CDN_BASE").cloned().filter(|c| !c.is_empty()),
            ipv6_mode: None,
        },
        source: path.to_path_buf(),
        needs_passphrase: key_path.is_some(),
    })
}

/// Разобрать файл вида `КЛЮЧ="значение"`.
///
/// Это не полноценная оболочка и быть ею не должна: подстановки команд и ветвления
/// в таком файле не встречаются, а выполнять его содержимое ради разбора значило бы
/// запустить чужой код ради четырёх строк настроек.
fn parse(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_start_matches("export ").trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }

        // Хвостовой комментарий отрезается только вне кавычек: в значении решётка
        // законна, и рубить по ней вслепую значит портить пути и пароли.
        let value = strip_value(rest.trim());
        out.insert(key.to_owned(), value);
    }
    out
}

fn strip_value(raw: &str) -> String {
    let raw = raw.trim();
    let (quote, body) = match raw.chars().next() {
        Some(q @ ('"' | '\'')) => (Some(q), &raw[q.len_utf8()..]),
        _ => (None, raw),
    };

    match quote {
        Some(q) => match body.find(q) {
            Some(end) => body[..end].to_owned(),
            None => body.to_owned(),
        },
        None => body
            .split_once('#')
            .map_or(body, |(v, _)| v)
            .trim()
            .to_owned(),
    }
}

/// Развернуть `$HOME` и `~` — в файле путь к ключу записан именно так.
fn expand_home(value: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return value.to_owned();
    }
    value
        .replace("$HOME", &home)
        .replace("${HOME}", &home)
        .replacen("~/", &format!("{home}/"), 1)
}
