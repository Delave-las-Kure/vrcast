//! T089 — сверка контрольных сумм перед вводом файла в раздачу (FR-032).
//!
//! Зачем это нужно, если передача идёт по надёжному соединению: надёжен канал,
//! а не всё вместе. Между локальным диском и серверным лежат ещё чтение файла,
//! запись по смещению с несколькими попытками, докачка после обрывов и файловая
//! система сервера. Каждое звено обычно не врёт — но «обычно» здесь недостаточно:
//! испорченный файл в раздаче ломает просмотр незаметно, и связать это с заливкой
//! месячной давности будет уже нечем.
//!
//! Сумма считается **на сервере его же средствами**, а не тем же кодом, которым
//! файл передавали. Сверять результат тем же средством, которым его получили, —
//! значит проверять, что код устойчиво повторяет свою же ошибку.

use crate::server::shell_quote;
use crate::ssh::{Connection, Result, SshError};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Сколько читаем за раз при подсчёте суммы локального файла.
const READ_CHUNK: usize = 1024 * 1024;

/// Посчитать сумму локального файла.
///
/// Файл может быть на десятки гигабайт, поэтому читается кусками, а не целиком
/// в память. Работа блокирующая и потому уносится с исполнителя задач: иначе она
/// на минуты займёт поток, на котором идут все остальные задачи.
pub async fn local(path: &Path) -> std::io::Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;

        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            let read = file.read(&mut buf)?;
            if read == 0 {
                break;
            }
            hasher.update(&buf[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|e| std::io::Error::other(e.to_string()))?
}

/// Посчитать сумму файла на сервере средствами самого сервера.
pub async fn remote(conn: &Connection, path: &str) -> Result<String> {
    let out = conn
        .exec(&format!("sha256sum -- {}", shell_quote(path)))
        .await?;
    if !out.ok() {
        return Err(SshError::Exec(format!(
            "не посчитать контрольную сумму на сервере: {}",
            out.stderr.trim()
        )));
    }

    // Вывод: «<сумма>  <имя>». Берём первое слово.
    out.trimmed()
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| {
            SshError::Exec(format!(
                "вывод sha256sum не разобрать: «{}»",
                out.trimmed().chars().take(80).collect::<String>()
            ))
        })
}

/// Совпадают ли суммы. Сравнение без учёта регистра: разные средства пишут
/// шестнадцатеричные числа по-разному, и различие в регистре — не расхождение.
pub fn matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
