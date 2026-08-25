//! T030 — медиа и файл раздачи (`data-model.md` §3–4), включая правила `slug`.
//!
//! Почему у файла нет полей `origin_url` и `cdn_url`, хотя они есть в модели данных:
//! ссылка **вычисляется** из профиля сервера (см. `links`), а не хранится рядом с
//! файлом. Сохранённая ссылка молча устареет в тот день, когда пользователь сменит
//! домен или подключит CDN, — и приложение начнёт выдавать нерабочие адреса, о чём
//! никто не узнает, пока их не откроет зритель.

use serde::{Deserialize, Serialize};

/// Предел длины `slug`. Имя файла складывается как `<slug>_<битрейт>.mp4`, а предел
/// имени файла в файловых системах — 255 байт; сотня оставляет запас на суффиксы
/// и на то, что не-латинские имена в кодировке занимают больше одного байта на знак.
pub const MAX_SLUG_LEN: usize = 100;

/// Медиа — то, что пользователь считает одним произведением.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub id: String,
    pub title: String,
    pub slug: String,
    /// Имена файлов раздачи, относительно каталога видео.
    ///
    /// Умолчание задано намеренно: опись лежит на сервере и её мог править человек,
    /// а медиа без единого файла — это законное состояние, а не повод не прочитать
    /// опись целиком.
    #[serde(default)]
    pub files: Vec<String>,
    /// Описания наборов качеств, относительно каталога видео.
    #[serde(default)]
    pub ladders: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    /// Поля, которых это приложение не знает.
    ///
    /// Сохраняются при перезаписи описи намеренно — по той же причине, что и на
    /// уровне всей описи (см. `manifest::Manifest::extra`): медиа мог завести более
    /// новый экземпляр приложения, и выбросить его сведения значит их потерять.
    #[serde(
        flatten,
        default,
        skip_serializing_if = "std::collections::HashMap::is_empty"
    )]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl Media {
    /// Новое медиа без файлов.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        slug: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            slug: slug.into(),
            files: Vec::new(),
            ladders: Vec::new(),
            created_at: created_at.into(),
            extra: std::collections::HashMap::new(),
        }
    }

    /// Все пути, числящиеся за медиа: и файлы, и описания наборов качеств.
    pub fn all_paths(&self) -> impl Iterator<Item = &String> {
        self.files.iter().chain(self.ladders.iter())
    }
}

/// Файл раздачи: известные о нём факты.
///
/// Всё, кроме `path`, `size_bytes` и `exists_on_server`, может быть неизвестно —
/// параметры добываются разбором заголовка MP4, и у файла, подготовленного не нашим
/// процессом, заголовка в начале может не оказаться (см. `moov`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFile {
    /// Путь относительно каталога видео.
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Средний битрейт.
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// `moov` найден в начале файла. Ложь = файл не соответствует целевому формату,
    /// и зритель будет ждать скачивания хвоста перед началом воспроизведения.
    /// `None` = ещё не проверяли.
    pub faststart_ok: Option<bool>,
    /// Ложь = файл удалён или переименован мимо приложения (FR-018).
    pub exists_on_server: bool,
}

impl MediaFile {
    /// Файл, о котором известно только то, что он есть и сколько весит.
    pub fn known(path: impl Into<String>, size_bytes: u64) -> Self {
        Self {
            path: path.into(),
            size_bytes,
            duration_s: None,
            width: None,
            height: None,
            bitrate_bps: None,
            video_codec: None,
            audio_codec: None,
            faststart_ok: None,
            exists_on_server: true,
        }
    }

    /// Годится ли ссылка на этот файл к выдаче зрителю (FR-018).
    pub fn link_is_usable(&self) -> bool {
        self.exists_on_server
    }
}

/// Что не так со `slug`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    Empty,
    TooLong { len: usize },
    BadChars { first_bad: char },
    Reserved,
}

impl std::fmt::Display for SlugError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("Короткое имя не может быть пустым."),
            Self::TooLong { len } => write!(
                f,
                "Короткое имя длиной {len} знаков не поместится в имя файла — сократите до {MAX_SLUG_LEN}."
            ),
            Self::BadChars { first_bad } => write!(
                f,
                "Знак «{first_bad}» в коротком имени недопустим: разрешены латинские буквы, цифры, дефис и подчёркивание."
            ),
            Self::Reserved => f.write_str(
                "Такое короткое имя занято служебным назначением — выберите другое.",
            ),
        }
    }
}

/// Имена, которые нельзя занимать: они значат для файловой системы или для раздачи
/// не то, что видно на глаз.
const RESERVED_SLUGS: &[&str] = &["_slow", ".", ".."];

/// Проверить `slug`: латинские буквы, цифры, дефис, подчёркивание (`data-model.md` §3).
pub fn validate_slug(slug: &str) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if let Some(bad) = slug
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Err(SlugError::BadChars { first_bad: bad });
    }
    // Длина считается после проверки знаков: сообщение про недопустимый знак
    // полезнее, чем про длину, когда неверно и то и другое.
    if slug.len() > MAX_SLUG_LEN {
        return Err(SlugError::TooLong { len: slug.len() });
    }
    if RESERVED_SLUGS.contains(&slug) {
        return Err(SlugError::Reserved);
    }
    Ok(())
}

/// Составить `slug` из названия.
///
/// Названия у пользователя русские, а `slug` попадает в имя файла и в ссылку, поэтому
/// кириллица переводится в латиницу. Возвращает `None`, если переводить нечего
/// (название целиком из знаков, у которых нет латинского соответствия) — тогда
/// короткое имя обязан задать человек, а не приложение из мусора.
pub fn slugify(title: &str) -> Option<String> {
    let mut out = String::with_capacity(title.len());
    let mut pending_separator = false;

    for ch in title.chars().flat_map(|c| c.to_lowercase()) {
        if let Some(latin) = transliterate(ch) {
            if !latin.is_empty() {
                if pending_separator && !out.is_empty() {
                    out.push('-');
                }
                pending_separator = false;
                out.push_str(latin);
            }
        } else if ch.is_ascii_alphanumeric() {
            if pending_separator && !out.is_empty() {
                out.push('-');
            }
            pending_separator = false;
            out.push(ch);
        } else {
            // Любой прочий знак — разделитель. Разделители не копятся: подряд идущие
            // пробелы, точки и тире дают один дефис, а не цепочку.
            pending_separator = true;
        }
    }

    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        return None;
    }

    let capped = cap_len(&trimmed, MAX_SLUG_LEN);
    if capped.is_empty() {
        None
    } else {
        Some(capped)
    }
}

/// Обрезать до предела, не разрывая слово посередине, если этого можно избежать.
fn cap_len(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let cut = &s[..max];
    match cut.rfind('-') {
        // Обрезаем по границе слова, но только если так остаётся хотя бы половина
        // предела: иначе от длинного названия останется огрызок.
        Some(i) if i >= max / 2 => cut[..i].to_owned(),
        _ => cut.trim_end_matches('-').to_owned(),
    }
}

/// Латинское соответствие для кириллической буквы.
///
/// `Some("")` — буква, которая пишется пустотой (твёрдый и мягкий знаки).
/// `None` — не кириллица, решает вызывающий.
fn transliterate(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sch",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        // Украинские и белорусские буквы: пользователь может назвать так же.
        'і' => "i",
        'ї' => "yi",
        'є' => "ye",
        'ґ' => "g",
        'ў' => "u",
        _ => return None,
    })
}
