//! T119 — каким кодировщиком пользоваться (FR-026).
//!
//! Три правила, и третье — самое важное:
//!
//! 1. пользоваться аппаратным, когда он есть;
//! 2. работать без него;
//! 3. **не молчать о переходе на процессор**.
//!
//! Третье не вежливость. Разница во времени — разы: то, что видеокарта делает
//! за десять минут, процессор делает час-полтора. Человек, не предупреждённый
//! об этом, решит, что приложение зависло, и убьёт задачу на середине.
//!
//! А вот о качестве беспокоиться не надо, и это не общие слова, а замер
//! 2026-08-02 на своём материале: программный x264 против NVENC дал разницу
//! +1.13 по шкале VMAF на четырёх мегабитах и ноль (даже небольшой минус)
//! на рабочих битрейтах от четырнадцати и выше. Поэтому в сообщении о переходе
//! честно сказано: потеряете время, а не качество.

use crate::domain::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// Чем кодировать.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Encoder {
    /// Аппаратный кодировщик видеокарты или процессора.
    Hardware { name: String },
    /// Программный x264. Медленнее в разы, но по качеству на рабочих битрейтах
    /// не уступает.
    Software,
}

impl Encoder {
    /// Имя, которое понимает FFmpeg.
    pub fn ffmpeg_name(&self) -> &str {
        match self {
            Self::Hardware { name } => name,
            Self::Software => "libx264",
        }
    }
}

/// Что выбрали и что об этом сказать человеку.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncoderChoice {
    pub encoder: Encoder,
    /// What to say about the choice, if anything. Empty means there is nothing to
    /// warn about: the best available was taken and nobody loses anything.
    pub notice: Option<Detail>,
}

/// Почему выбрать не удалось.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("nothing to encode with: the bundled build has neither a hardware H.264 encoder nor a software one")]
pub struct NoEncoder;

/// Порядок предпочтения.
///
/// NVIDIA первой — она быстрее прочих на нашем материале. Дальше встроенное
/// в процессор Intel, потом AMD, и последним общий путь Linux: он работает
/// и с Intel, и с AMD, но выбирать его при наличии своего смысла нет.
const PREFERRED: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi"];

/// Выбрать кодировщик.
///
/// `available` — что **умеет звать** вложенная сборка (см. `ffmpeg::probe_self`).
/// Это не то же самое, что «работает на этой машине»: наличие кодировщика в сборке
/// ничего не говорит о железе, и настоящий ответ даёт только пробный запуск.
/// Поэтому выбор здесь — предположение, а проверка его — отдельный шаг.
///
/// `prefer_hardware` = ложь, когда человек сам попросил процессор.
pub fn choose(
    available: &[String],
    has_x264: bool,
    prefer_hardware: bool,
) -> Result<EncoderChoice, NoEncoder> {
    if prefer_hardware {
        if let Some(name) = PREFERRED
            .iter()
            .find(|p| available.iter().any(|a| a.eq_ignore_ascii_case(p)))
        {
            return Ok(EncoderChoice {
                encoder: Encoder::Hardware {
                    name: (*name).to_owned(),
                },
                notice: None,
            });
        }
    }

    if !has_x264 {
        return Err(NoEncoder);
    }

    Ok(EncoderChoice {
        encoder: Encoder::Software,
        notice: Some(Detail::new(if prefer_hardware {
            DetailCode::NoticeNoHardwareFound
        } else {
            DetailCode::NoticeSoftwareAsAsked
        })),
    })
}

/// Что сказать, когда аппаратный кодировщик подвёл на деле.
///
/// Наличие в сборке не значит работоспособности: у видеокарты может не быть нужного
/// блока, драйвер может быть старым, а на ноутбуке видеокарта — просто отключённой.
/// Переход на процессор в этом случае — правильное поведение, но молчать о нём
/// нельзя вдвойне: человек ждал десяти минут, а получит час.
pub fn fallback_notice(failed: &str) -> Detail {
    Detail::new(DetailCode::NoticeHardwareFailed).with("encoder", failed.to_string())
}
