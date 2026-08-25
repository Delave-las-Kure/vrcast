//! T108 — исходный файл и его дорожки (data-model §6, FR-020, FR-021).
//!
//! Здесь только описание того, что нашлось в файле, и правила показа. Ничего
//! из этого не решает, что с файлом делать: решение живёт в [`super::convert_plan`],
//! и разделение намеренное — разбор исходника случается один раз, а план пересчитывается
//! на каждое движение ползунка.

use serde::{Deserialize, Serialize};

/// Звуковая дорожка исходника.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// Порядковый номер среди звуковых дорожек, с нуля: именно его понимает ffmpeg
    /// в `-map 0:a:<N>`.
    pub index: usize,
    pub codec: String,
    pub channels: u16,
    /// Битрейт дорожки, если известен.
    pub bitrate_bps: Option<u64>,
    /// Язык. Часто отсутствует — и это обычное дело, а не поломка.
    pub language: Option<String>,
    /// Название дорожки: «Дубляж», «Оригинал», «Комментарии режиссёра».
    pub title: Option<String>,
    pub is_default: bool,
}

impl AudioTrack {
    /// Как назвать дорожку человеку.
    ///
    /// Язык есть не всегда — у многих раздач он просто не проставлен. Показывать
    /// в таком случае пустоту нельзя: выбирать между двумя пустыми строками
    /// невозможно, а дорожек бывает шесть. Поэтому порядковый номер — не запасной
    /// вариант, а полноценный ответ (граничный случай спеки к FR-020).
    pub fn label(&self) -> String {
        let основа = match (&self.language, &self.title) {
            (Some(lang), Some(title)) if !lang.is_empty() && !title.is_empty() => {
                format!("{lang} — {title}")
            }
            (Some(lang), _) if !lang.is_empty() => lang.clone(),
            (_, Some(title)) if !title.is_empty() => title.clone(),
            // Номер показывается человеку с единицы: «дорожка 0» читается как ошибка.
            _ => format!("Дорожка {}", self.index + 1),
        };

        let каналы = match self.channels {
            0 => String::new(),
            1 => String::from(", моно"),
            2 => String::from(", стерео"),
            n => format!(", {n} каналов"),
        };
        format!("{основа}{каналы}")
    }
}

/// Разобранный исходник.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: f64,
    pub width: u32,
    pub height: u32,
    /// Кадров в секунду, округлённых **вверх**: 47.952 — это 48-кадровый материал,
    /// и округление вниз занизило бы уровень совместимости.
    pub fps: u32,
    pub bitrate_bps: u64,
    /// Пиковый битрейт, если его мерили. Замер стоит времени и делается отдельно.
    pub peak_bps: Option<u64>,
    pub video_codec: String,
    pub pix_fmt: String,
    /// Характеристика передачи цвета. По ней узнаётся HDR, который надо приводить
    /// к обычному диапазону, иначе картинка у зрителя выйдет блёклой.
    pub color_transfer: Option<String>,
    pub audio_tracks: Vec<AudioTrack>,
}

/// Признаки HDR в характеристике передачи цвета.
const HDR_TRANSFERS: [&str; 4] = ["smpte2084", "arib-std-b67", "smpte428", "bt2020-10"];

impl SourceFile {
    /// Дорожка, которую стоит предложить по умолчанию.
    ///
    /// Помеченная как основная, иначе первая. Пусто — только если звука нет вовсе;
    /// это отдельный случай, а не «возьмём нулевую» (FR-021, код `NO_AUDIO_TRACKS`).
    pub fn default_track(&self) -> Option<&AudioTrack> {
        self.audio_tracks
            .iter()
            .find(|t| t.is_default)
            .or_else(|| self.audio_tracks.first())
    }

    pub fn track(&self, index: usize) -> Option<&AudioTrack> {
        self.audio_tracks.iter().find(|t| t.index == index)
    }

    /// Записан ли исходник в расширенном динамическом диапазоне.
    ///
    /// Важно не само по себе: такую картинку нужно приводить к обычному диапазону,
    /// а значит копировать поток без пересжатия уже нельзя.
    pub fn is_hdr(&self) -> bool {
        match &self.color_transfer {
            Some(t) => {
                let t = t.to_ascii_lowercase();
                HDR_TRANSFERS.iter().any(|h| t == *h)
            }
            None => false,
        }
    }

    /// Сколько пикселей в кадре.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}
