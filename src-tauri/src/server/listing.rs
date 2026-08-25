//! T044 — что лежит в каталоге раздачи.
//!
//! Читается **верхний уровень**: файл — запись, каталог — тоже запись (обычно это
//! набор качеств). Спускаться внутрь набора незачем: пользователь думает о нём как
//! о единице, а показывать ему каждый отрезок значило бы утопить библиотеку в шуме.
//!
//! Перечень не фильтруется: он честно отдаёт всё, что видно на сервере. Решение,
//! что из этого показывать, принимается выше — иначе фильтр пришлось бы помнить
//! в каждом месте, где перечень используется.

use crate::ssh::{Connection, Result, SshError};

/// Одна запись каталога раздачи.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Имя относительно каталога раздачи.
    pub name: String,
    /// Размер: для каталога — суммарный размер того, что внутри.
    pub size_bytes: u64,
    pub is_dir: bool,
}

/// Прочитать содержимое каталога раздачи.
pub async fn list(conn: &Connection, video_dir: &str) -> Result<Vec<Entry>> {
    let sftp = conn.sftp().await?;

    let entries = sftp
        .read_dir(video_dir)
        .await
        .map_err(|e| SshError::Sftp(crate::store::redact::safe_display(&e)))?;

    let mut out = Vec::new();
    for e in entries {
        let name = e.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let meta = e.metadata();
        out.push(Entry {
            name,
            size_bytes: meta.size.unwrap_or(0),
            is_dir: meta.is_dir(),
        });
    }

    // Каталоги в перечне пришли с размером самой записи каталога, а не того, что
    // внутри. Досчитываем одной командой на все сразу: отдельный обход по каждому
    // набору качеств — это десятки обращений к серверу там, где хватает одного.
    let dirs: Vec<&str> = out
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.as_str())
        .collect();
    if !dirs.is_empty() {
        let sizes = directory_sizes(conn, video_dir, &dirs).await?;
        for entry in out.iter_mut().filter(|e| e.is_dir) {
            if let Some(size) = sizes.get(entry.name.as_str()) {
                entry.size_bytes = *size;
            }
        }
    }

    // Порядок устойчивый: перечень видит человек, и он не должен прыгать
    // от обращения к обращению.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Размеры перечисленных каталогов — одной командой.
async fn directory_sizes(
    conn: &Connection,
    video_dir: &str,
    dirs: &[&str],
) -> Result<std::collections::HashMap<String, u64>> {
    let args = dirs
        .iter()
        .map(|d| super::shell_quote(&super::join_remote(video_dir, d)))
        .collect::<Vec<_>>()
        .join(" ");
    let out = conn.exec(&format!("du -sb -- {args} 2>/dev/null")).await?;

    let mut map = std::collections::HashMap::new();
    for line in out.stdout.lines() {
        let Some((size, path)) = line.split_once('\t') else {
            continue;
        };
        let Ok(size) = size.trim().parse::<u64>() else {
            continue;
        };
        // Имя каталога — последний отрезок пути. Сравнивать с исходным именем
        // надёжнее, чем полагаться на порядок строк вывода.
        let name = path.trim().rsplit('/').next().unwrap_or("").to_owned();
        if !name.is_empty() {
            map.insert(name, size);
        }
    }
    Ok(map)
}
