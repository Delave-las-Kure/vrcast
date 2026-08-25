//! T050 — сколько места на диске сервера (FR-017).
//!
//! Показывается не из любопытства: заливка на переполненный диск обрывается
//! на середине, и узнать об этом лучше до, а не после часа передачи (FR-036).

use crate::commands::library::DiskUsage;
use crate::ssh::{Connection, Result, SshError};

/// Прочитать состояние диска, на котором лежит каталог раздачи.
pub async fn usage(conn: &Connection, video_dir: &str) -> Result<DiskUsage> {
    let dir = super::shell_quote(video_dir);

    // Одна команда вместо двух: каждое обращение к серверу — это оборот по сети,
    // а показать место надо вместе с библиотекой, а не через секунду после неё.
    //
    // `df -P` даёт предсказуемый вид вывода: одна строка на файловую систему,
    // без переносов длинных имён устройств. Размеры в килобайтах — так велит
    // POSIX, и это устойчивее, чем просить байты ключом, которого может не быть.
    let out = conn
        .exec(&format!(
            "df -Pk -- {dir} | tail -n 1; du -sk -- {dir} 2>/dev/null | cut -f1"
        ))
        .await?;

    if !out.ok() {
        return Err(SshError::Exec(format!(
            "не удалось узнать место на диске: {}",
            out.stderr.trim()
        )));
    }

    let mut lines = out.stdout.lines();
    let df_line = lines.next().unwrap_or_default();
    let du_line = lines.next().unwrap_or_default();

    // Вывод df: устройство, всего, занято, доступно, процент, точка монтирования.
    let fields: Vec<&str> = df_line.split_whitespace().collect();
    let total_kb = fields.get(1).and_then(|v| v.parse::<u64>().ok());
    let free_kb = fields.get(3).and_then(|v| v.parse::<u64>().ok());

    let (Some(total_kb), Some(free_kb)) = (total_kb, free_kb) else {
        return Err(SshError::Exec(format!(
            "вывод df не разобрать: «{}»",
            df_line.trim()
        )));
    };

    // Объём каталога раздачи — не обязателен: на очень большой библиотеке подсчёт
    // заметно дольше остального, и лучше показать место на диске без него,
    // чем не показать ничего.
    let used_kb = du_line.trim().parse::<u64>().unwrap_or(0);

    Ok(DiskUsage {
        total_bytes: total_kb.saturating_mul(1024),
        free_bytes: free_kb.saturating_mul(1024),
        used_by_videos_bytes: used_kb.saturating_mul(1024),
    })
}
