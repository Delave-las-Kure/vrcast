//! T116 — разбор исходника: что за файл нам дали (FR-020, FR-021).
//!
//! Спрашивается у `ffprobe` из той же вложенной сборки, что и всё прочее. Своего
//! разбора контейнеров здесь нет и не будет: их десятки, каждый со своими
//! странностями, и написать это лучше, чем уже написано, не выйдет.
//!
//! Разбор ответа отделён от запуска намеренно. Ответ `ffprobe` — это данные,
//! и все тонкости чтения (числа строками, `und` вместо отсутствующего языка,
//! частота кадров дробью) проверяются тестом на записанном ответе, без файла
//! на диске и без самой программы.

use super::ffmpeg;
use crate::domain::source::{AudioTrack, SourceFile};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),

    #[error("файл не разобрать: {0}")]
    Unreadable(String),

    #[error("в файле нет видео")]
    NoVideo,
}

pub type Result<T> = std::result::Result<T, ProbeError>;

/// Разобрать исходник.
pub async fn probe(path: &Path) -> Result<SourceFile> {
    let ffprobe = ffmpeg::locate("ffprobe")?;

    let out = tokio::process::Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| ProbeError::Unreadable(format!("{}: {e}", path.display())))?;

    if !out.status.success() {
        // Слово `ffprobe` человеку ничего не говорит, а вот его жалоба говорит:
        // «moov atom not found», «Invalid data found». Передаём как есть.
        let жалоба = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(ProbeError::Unreadable(if жалоба.is_empty() {
            format!("{} — разбор не удался без объяснений", path.display())
        } else {
            жалоба
        }));
    }

    let text = String::from_utf8_lossy(&out.stdout);
    parse(&text, &path.display().to_string())
}

// ---------- разбор ответа ----------

/// Ответ `ffprobe` в том виде, в каком он приходит.
///
/// Числа здесь строками не по недосмотру: `ffprobe` так их и печатает. Пытаться
/// прочитать их как числа значит получить отказ разбора на первом же файле.
#[derive(Debug, Deserialize)]
struct Probed {
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    format: Format,
}

#[derive(Debug, Default, Deserialize)]
struct Format {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    color_transfer: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    bit_rate: Option<String>,
    channels: Option<u16>,
    #[serde(default)]
    tags: Tags,
    #[serde(default)]
    disposition: Disposition,
}

#[derive(Debug, Default, Deserialize)]
struct Tags {
    language: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    default: u8,
}

/// Прочитать ответ `ffprobe`.
pub fn parse(json: &str, path: &str) -> Result<SourceFile> {
    let probed: Probed = serde_json::from_str(json)
        .map_err(|e| ProbeError::Unreadable(format!("ответ разборщика не прочитать: {e}")))?;

    let video = probed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or(ProbeError::NoVideo)?;

    let audio_tracks = probed
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .enumerate()
        .map(|(index, s)| AudioTrack {
            // Номер среди ЗВУКОВЫХ потоков, а не среди всех: именно его понимает
            // ffmpeg в `-map 0:a:<N>`. Взять общий номер потока значит промахнуться
            // дорожкой на любом файле, где звук идёт не первым.
            index,
            codec: s.codec_name.clone().unwrap_or_default(),
            channels: s.channels.unwrap_or(0),
            bitrate_bps: number(&s.bit_rate),
            language: language(&s.tags.language),
            title: not_empty(&s.tags.title),
            is_default: s.disposition.default == 1,
        })
        .collect();

    Ok(SourceFile {
        path: path.to_owned(),
        size_bytes: number(&probed.format.size).unwrap_or(0),
        duration_s: probed
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .unwrap_or(0.0),
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        fps: fps(video),
        bitrate_bps: number(&video.bit_rate)
            .or_else(|| number(&probed.format.bit_rate))
            .unwrap_or(0),
        peak_bps: None,
        video_codec: video.codec_name.clone().unwrap_or_default(),
        pix_fmt: video.pix_fmt.clone().unwrap_or_default(),
        color_transfer: not_empty(&video.color_transfer),
        audio_tracks,
    })
}

/// Частота кадров, округлённая **вверх**.
///
/// Приходит дробью: `24/1`, `24000/1001`. Округление вниз превратило бы 47.952
/// в 47 и занизило уровень совместимости — а занижённый уровень строгий декодер
/// вправе не принять.
fn fps(video: &Stream) -> u32 {
    let source = video
        .r_frame_rate
        .as_deref()
        .filter(|s| *s != "0/0")
        .or(video.avg_frame_rate.as_deref())
        .unwrap_or("");

    let (num, den) = match source.split_once('/') {
        Some((n, d)) => (n.parse::<u64>().ok(), d.parse::<u64>().ok()),
        None => (source.parse::<u64>().ok(), Some(1)),
    };

    match (num, den) {
        (Some(n), Some(d)) if d > 0 && n > 0 => n.div_ceil(d) as u32,
        // Ноль кадров в секунду не бывает. Тридцать — безобидная догадка: она
        // завышает уровень совместимости, а завышенный безопасен всегда.
        _ => 30,
    }
}

fn number(s: &Option<String>) -> Option<u64> {
    s.as_deref().and_then(|v| v.parse::<u64>().ok())
}

fn not_empty(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Язык дорожки.
///
/// `und` — это «не указано», а не название языка. Показать его человеку значит
/// предложить выбирать между «und» и «und»; порядковый номер в этом случае
/// полезнее (граничный случай спеки к FR-020).
fn language(raw: &Option<String>) -> Option<String> {
    not_empty(raw).filter(|v| !v.eq_ignore_ascii_case("und"))
}
