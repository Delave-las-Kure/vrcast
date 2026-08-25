//! T010 — вырезание секретов из всего, что приложение выводит наружу.
//!
//! Конституция, принцип IV: «Секреты MUST NOT попадать в журналы, отчёты об ошибках и
//! диагностические выгрузки». Полагаться на аккуратность каждой строчки кода здесь нельзя:
//! достаточно одного `tracing::debug!("{:?}", profile)` где-нибудь в глубине, чтобы пароль
//! от чужого сервера оказался в файле журнала. Поэтому защита стоит не в местах вывода,
//! а на самом выходе — через него проходит всё.
//!
//! Работает это так: любой секрет, попадающий в приложение, регистрируется здесь (это делает
//! `store::secrets`, а не вызывающий код), и дальше любая строка, уходящая в журнал, проходит
//! замену перед записью.

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::{OnceLock, RwLock};

/// Чем заменяется найденный секрет. Намеренно заметно: если это появилось в журнале,
/// значит защита сработала, а не «строка просто выглядит странно».
pub const MASK: &str = "[секрет скрыт]";

/// Короче этого не регистрируем. Секрет из трёх символов встретится в журнале случайно
/// сотни раз и превратит его в решето из масок, скрыв заодно всё полезное.
const MIN_SECRET_LEN: usize = 8;

fn registry() -> &'static RwLock<Vec<String>> {
    static REGISTRY: OnceLock<RwLock<Vec<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Запомнить значение, которое не должно появляться в выводе.
///
/// Вызывается там, где секрет входит в приложение, — не там, где он выводится.
/// Слишком короткие значения игнорируются намеренно (см. `MIN_SECRET_LEN`).
pub fn register(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    let Ok(mut list) = registry().write() else {
        return; // отравленная блокировка: молча не регистрируем, но и не роняем приложение
    };
    if list.iter().any(|s| s == secret) {
        return;
    }
    list.push(secret.to_owned());
    // Длинные вперёд: если один секрет содержит другой, замена начнётся с длинного,
    // иначе от него останется хвост.
    list.sort_by_key(|b| std::cmp::Reverse(b.len()));
}

/// Забыть ОДИН секрет — тот, что больше не существует.
///
/// Именно так и надо забывать при удалении профиля. Соблазн позвать здесь
/// `forget_all` силён и опасен: у остальных профилей секреты живы, и снятая
/// с них маскировка означала бы, что чужой пароль попадёт в журнал при первой же
/// ошибке — до перезапуска приложения и без единого признака, что что-то не так
/// (задолженность T073, принцип IV).
pub fn forget(secret: &str) {
    if secret.len() < MIN_SECRET_LEN {
        return;
    }
    if let Ok(mut list) = registry().write() {
        list.retain(|s| s != secret);
    }
}

/// Забыть все зарегистрированные секреты.
///
/// **Только для тестов.** В работающем приложении звать это нельзя: секреты
/// остальных профилей от этого перестанут вырезаться из вывода. Для удаления
/// одного профиля есть [`forget`].
pub fn forget_all() {
    if let Ok(mut list) = registry().write() {
        list.clear();
    }
}

/// Заменить в тексте все зарегистрированные секреты и блоки приватных ключей.
///
/// Возвращает `Cow`, чтобы в обычном случае (секретов в тексте нет) не копировать строку:
/// через эту функцию проходит каждая строка журнала.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = redact_key_blocks(text);

    if let Ok(list) = registry().read() {
        for secret in list.iter() {
            if out.contains(secret.as_str()) {
                out = Cow::Owned(out.replace(secret.as_str(), MASK));
            }
        }
    }
    out
}

/// Метки блока приватного ключа. Общие для замены и для построчного писателя:
/// он обязан узнавать начатый блок, чтобы не выпустить его тело по частям.
const KEY_BEGIN: &str = "-----BEGIN";
const KEY_END: &str = "-----END";

/// Приватный ключ узнаётся по форме, а не по регистрации.
///
/// Это подстраховка на случай, когда ключ прочитан из файла и попал в вывод, минуя
/// хранилище секретов — то есть ровно в том случае, когда регистрация не сработала.
fn redact_key_blocks(text: &str) -> Cow<'_, str> {
    const BEGIN: &str = KEY_BEGIN;
    const END_MARK: &str = KEY_END;

    if !text.contains(BEGIN) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find(BEGIN) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];

        // Ищем конец блока: "-----END ... -----"
        match tail.find(END_MARK).and_then(|e| {
            tail[e + END_MARK.len()..]
                .find("-----")
                .map(|d| e + END_MARK.len() + d + 5)
        }) {
            Some(end) => {
                out.push_str(MASK);
                rest = &tail[end..];
            }
            None => {
                // Начало блока есть, конца нет — обрезанный вывод. Прячем весь остаток:
                // лучше потерять хвост журнала, чем показать половину ключа.
                out.push_str(MASK);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Безопасное представление ошибки для показа и записи.
///
/// Ошибки — второй по частоте путь утечки после журнала: секрет попадает в сообщение
/// через `{}` от нижележащей библиотеки, которая о наших правилах ничего не знает.
pub fn safe_display<E: std::fmt::Display + ?Sized>(err: &E) -> String {
    redact(&err.to_string()).into_owned()
}

/// Обёртка над потоком вывода, вырезающая секреты перед записью.
///
/// Накапливает вывод построчно: запись может прийти по частям, и секрет, разорванный
/// между двумя вызовами, иначе прошёл бы мимо замены.
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            buf: Vec::new(),
        }
    }

    fn flush_complete_lines(&mut self) -> io::Result<()> {
        loop {
            let Some(pos) = self.buf.iter().position(|b| *b == b'\n') else {
                return Ok(());
            };

            let first_line = String::from_utf8_lossy(&self.buf[..=pos]);
            if !first_line.contains(KEY_BEGIN) {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line);
                self.inner.write_all(redact(&text).as_bytes())?;
                continue;
            }

            // Строка начинает блок приватного ключа. Выпускать его построчно нельзя:
            // тело ключа — строки чистого base64, по отдельности неотличимые от
            // безобидного вывода, и узнать блок можно только целиком. Пока конец блока
            // не пришёл — держим буфер (flush замаскирует остаток как обрезанный блок).
            if has_unclosed_key_block(&String::from_utf8_lossy(&self.buf)) {
                return Ok(());
            }

            // Конец блока в буфере: выпускаем все накопленные полные строки одним
            // куском через одну замену — она видит блок целиком и маскирует его.
            let last_nl = self.buf.iter().rposition(|b| *b == b'\n').unwrap_or(pos);
            let chunk: Vec<u8> = self.buf.drain(..=last_nl).collect();
            let text = String::from_utf8_lossy(&chunk);
            self.inner.write_all(redact(&text).as_bytes())?;
        }
    }
}

/// Есть ли в тексте начатый, но ещё не закрытый блок приватного ключа.
fn has_unclosed_key_block(text: &str) -> bool {
    let Some(start) = text.rfind(KEY_BEGIN) else {
        return false;
    };
    let tail = &text[start..];
    match tail.find(KEY_END) {
        Some(e) => !tail[e + KEY_END.len()..].contains("-----"),
        None => true,
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.flush_complete_lines()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_complete_lines()?;
        if !self.buf.is_empty() {
            let tail = std::mem::take(&mut self.buf);
            let text = String::from_utf8_lossy(&tail);
            self.inner.write_all(redact(&text).as_bytes())?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// Источник потоков вывода для `tracing`, оборачивающий каждый в вырезание секретов.
pub struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> tracing_subscriber::fmt::MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: tracing_subscriber::fmt::MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }
}
