//! T032 — зрительские ссылки (FR-016).
//!
//! Раздача отдаёт файлы из каталога видео по пути `/videos/…` — так устроен рабочий
//! сервер, и приложение обязано выдавать те же ссылки, что работают сейчас.
//!
//! Две ссылки, а не одна: origin отдаёт сам сервер, CDN — посредник. Когда CDN задан,
//! выбор оставляется человеку (FR-016), потому что у вариантов разная цена: origin
//! в России не блокируется, CDN быстрее, но зависит от посредника.

/// Часть пути раздачи, под которой лежит каталог видео.
pub const VIDEOS_PREFIX: &str = "videos";

/// Готовые ссылки на один файл.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Links {
    /// Ссылка через сам сервер. Есть всегда.
    pub origin: String,
    /// Ссылка через CDN. Нет, если CDN не задан в профиле.
    pub cdn: Option<String>,
}

impl Links {
    /// Ссылка по умолчанию — та, что работает без посредников.
    pub fn preferred(&self) -> &str {
        &self.origin
    }
}

/// Построить ссылки на файл раздачи.
///
/// `rel_path` — путь относительно каталога видео: `Backrooms_22.mp4` или
/// `backrooms/master.m3u8`. `domain` ожидается уже приведённым
/// (см. `server_profile::normalize_domain`), но схема и хвостовая косая снимаются
/// и здесь: эта функция вызывается и с данными, пришедшими из базы от прошлых версий.
pub fn for_path(domain: &str, cdn_base: Option<&str>, rel_path: &str) -> Links {
    let host = super::server_profile::normalize_domain(domain);
    let path = encode_path(rel_path);

    Links {
        origin: format!("https://{host}/{VIDEOS_PREFIX}/{path}"),
        cdn: cdn_base
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(|base| {
                let base = base.trim_end_matches('/');
                format!("{base}/{VIDEOS_PREFIX}/{path}")
            }),
    }
}

/// Закодировать путь для ссылки, сохранив разделители каталогов.
///
/// Кодировать нужно не из педантизма: имена файлов на сервере бывают какими угодно —
/// с пробелами, кириллицей, знаком решётки. Незакодированная решётка превращает
/// остаток имени в якорь, и ссылка ведёт в никуда, причём молча.
fn encode_path(rel_path: &str) -> String {
    rel_path
        .trim_matches('/')
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Процентное кодирование одного отрезка пути.
///
/// Без изменений остаются только «неограниченные» знаки (RFC 3986): латинские буквы,
/// цифры и `-._~`. Всё прочее кодируется. Это чуть строже необходимого, зато не
/// требует помнить, какие знаки безопасны в какой части ссылки; в обычном имени вида
/// `Backrooms_22.mp4` кодировать нечего, и ссылка остаётся читаемой.
fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}
