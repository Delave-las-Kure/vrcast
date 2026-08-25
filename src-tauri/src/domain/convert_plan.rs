//! T109, T110 — что делать с исходником: переносить или пересжимать (FR-022, FR-025, FR-029).
//!
//! Целевой формат один и не обсуждается: MP4, видео H.264 в yuv420p, звук AAC-LC
//! стерео, служебные данные в начале файла. Он выбран не из вкуса — его принимает
//! плеер VRChat, а всё прочее у части зрителей не играет вовсе.
//!
//! **Почему H.264, а не HEVC.** HEVC экономит 35–45 % битрейта, но требует декодера
//! у зрителя, а в Windows 10/11 системного HEVC нет: нужен отдельный пакет из магазина,
//! и плеер идёт через Media Foundation, то есть без пакета не играет ничего, сколько бы
//! видеокарта ни умела. Проверено боем 2026-07-30: **четверо зрителей из восьми
//! не смогли смотреть**. Поэтому исходник в HEVC здесь пересжимается, а не переносится,
//! — хотя формально «видео уже сжато» и копировать было бы дешевле.
//!
//! Правила ниже перенесены из `vrcast-convert` без изменения: каждое куплено ошибкой,
//! и переизобретение гарантированно повторило бы её (R-13).

use super::source::{AudioTrack, SourceFile};
use super::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// Целевой битрейт звука по умолчанию, килобит в секунду.
pub const AUDIO_KBPS: u32 = 256;

/// Допуск на бюджет звука.
///
/// Настоящий AAC стабильно чуть толще номинала: дорожка «128k» весит 128 634 бит.
/// Без допуска она уходила бы на пересжатие, теряя поколение впустую.
const AUDIO_TOLERANCE_PERCENT: u64 = 10;

/// Насколько потолок битрейта выше цели.
///
/// Было +30 % — и давало пик в 1.36 раза выше цели: за ступенью «35 Мбит/с» скрывалось
/// требование почти в 50. Замер 2026-08-02: снижение до +10 % стоило около 0.5 dB
/// и сняло 15 % требований к каналу зрителя.
const MAXRATE_PERCENT: u32 = 110;

/// Во сколько раз настоящий пик выше заданного потолка.
///
/// Потолок ограничивает не мгновенный битрейт, а среднее по окну проверки буфера,
/// и пик стабильно выходит на 5–6 % выше. Число в сотых долях, чтобы обойтись
/// целыми: нужен пик P — ставь потолок P/1.06.
const PEAK_OVER_MAXRATE: u32 = 106;

/// Что делать с видеопотоком.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoAction {
    /// Перенести как есть — ноль потерь и минуты вместо часов.
    Copy,
    /// Пересжать без заданного битрейта: «визуально без потерь».
    Reencode {
        /// Why it could not simply be carried across. Shown to a person: re-encoding
        /// takes hours, and they are entitled to know what they are paying for.
        reason: Detail,
        level: String,
    },
    /// Пересжать под заданный битрейт с ограничением пиков.
    ReencodeCapped {
        reason: Detail,
        level: String,
        target_kbps: u32,
        maxrate_kbps: u32,
        bufsize_kbps: u32,
    },
}

/// Что делать со звуком.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioAction {
    Copy,
    Reencode {
        reason: Detail,
        bitrate_kbps: u32,
        /// Выравнивание звука относительно картинки.
        ///
        /// Обязательно при пересжатии: AAC пишет вступительные отсчёты через список
        /// правок, а плеер VRChat его не читает — и звук уезжает. Это и есть FR-024,
        /// и без этого поля план был бы неполным.
        resample_fix: bool,
    },
}

/// Что помешало составить план.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanProblem {
    /// Звука нет вовсе — выбирать нечего.
    NoAudioTracks,
    /// Указанной дорожки в файле нет.
    NoSuchTrack { index: usize, available: usize },
    /// Задана нулевая высота кадра.
    HeightZero,
    /// Просят больше строк, чем есть в источнике.
    HeightAboveSource { asked: u32, source: u32 },
    /// Задан нулевой битрейт.
    BitrateZero,
    /// Просят битрейт заметно выше исходного.
    BitrateAboveSource { asked_kbps: u32, source_kbps: u64 },
}

impl PlanProblem {
    /// What to say about it. The wording belongs to the interface (FR-105, FR-106).
    ///
    /// Contradictions used to carry a ready sentence built where they were detected,
    /// which meant the same complaint could be worded two ways depending on which
    /// check raised it. A code cannot drift like that.
    pub fn detail(&self) -> Detail {
        match self {
            Self::NoAudioTracks => Detail::new(DetailCode::PlanNoAudioTracks),
            // Tracks are counted from one for a person and from zero for ffmpeg. The
            // conversion happens here, once, instead of in each catalogue entry.
            Self::NoSuchTrack { index, available } => Detail::new(DetailCode::PlanNoSuchTrack)
                .with("number", index + 1)
                .with("available", *available),
            Self::HeightZero => Detail::new(DetailCode::PlanHeightZero),
            Self::HeightAboveSource { asked, source } => {
                Detail::new(DetailCode::PlanHeightAboveSource)
                    .with("asked", *asked)
                    .with("source", *source)
            }
            Self::BitrateZero => Detail::new(DetailCode::PlanBitrateZero),
            Self::BitrateAboveSource {
                asked_kbps,
                source_kbps,
            } => Detail::new(DetailCode::PlanBitrateAboveSource)
                .with("asked_kbps", *asked_kbps)
                .with("source_kbps", *source_kbps),
        }
    }
}

/// Чего человек хочет от подготовки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertRequest {
    /// Какую звуковую дорожку взять.
    pub audio_track: usize,
    /// Целевой битрейт видео в килобитах. Пусто — не задавать, сжимать «без потерь
    /// на глаз».
    pub target_kbps: Option<u32>,
    /// Целевая высота кадра. Пусто — не менять.
    pub height: Option<u32>,
}

/// Готовый план подготовки.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertPlan {
    pub video: VideoAction,
    pub audio: AudioAction,
    /// Номер выбранной дорожки.
    pub audio_track: usize,
    /// Кадров между опорными.
    ///
    /// Опорный кадр раз в секунду при любой частоте кадров. Константа здесь была
    /// бы ошибкой: 48 писалось под 48-кадровое видео и означало «раз в секунду»,
    /// а на 24-кадровом давало раз в две.
    pub gop: u32,
    /// Приводить ли расширенный динамический диапазон к обычному.
    pub tonemap: bool,
    /// Requested frame height, as asked for. Kept even when it equals the source
    /// height: the command builder needs to tell "not asked" from "asked for the
    /// same", and only the former may skip the scaling filter.
    pub requested_height: Option<u32>,
    /// Служебные данные в начале файла — иначе зритель ждёт скачивания хвоста (FR-023).
    pub faststart: bool,
}

impl ConvertPlan {
    /// Останется ли качество нетронутым.
    pub fn lossless(&self) -> bool {
        self.video == VideoAction::Copy && self.audio == AudioAction::Copy
    }
}

/// Уровень совместимости H.264 по **двум** пределам сразу.
///
/// Проверять только размер кадра недостаточно, и это записанная ошибка: 1922×1082
/// при 48 кадрах — это 8228 макроблоков на кадр (почти влезает в 4.1) и 394 944
/// в секунду при пределе 4.1 в 245 760, то есть превышение в 1.6 раза. Занижённый
/// уровень строгий декодер вправе не принять; завышенный безопасен всегда.
pub fn h264_level(width: u32, height: u32, fps: u32) -> &'static str {
    // Макроблок — 16×16, и неполный тоже считается: 1922 даёт 121 столбец, а не 120.
    let mb = u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16));
    let mbps = mb * u64::from(fps.max(1));

    match () {
        _ if mb <= 8_192 && mbps <= 245_760 => "4.1",
        _ if mb <= 8_704 && mbps <= 522_240 => "4.2",
        _ if mb <= 22_080 && mbps <= 589_824 => "5.0",
        _ if mb <= 36_864 && mbps <= 983_040 => "5.1",
        _ => "5.2",
    }
}

/// Потолок и буфер под заданный целевой битрейт.
///
/// Возвращает килобиты. Считать в мегабитах нельзя, и это отдельная записанная
/// ошибка: при цели 8 Мбит/с целочисленное `8 * 11 / 10` даёт ровно 8 — потолок
/// совпадает с целью, буфера нет вовсе, и выходит режим постоянного битрейта,
/// который в замерах проиграл. На прежних +30 % это не вылезало (`8*13/10 = 10`),
/// а на +10 % сломалось молча.
///
/// Буфер равен потолку намеренно: большой буфер разрешает всплеск выше потолка,
/// и на этом зрители замирали — было `потолок 45 / буфер 60` и пики 54 Мбит/с.
pub fn peak_control(target_kbps: u32) -> (u32, u32) {
    let maxrate = target_kbps.saturating_mul(MAXRATE_PERCENT) / 100;
    // Потолок обязан быть строго выше цели: равенство и есть тот самый постоянный
    // битрейт, ради ухода от которого всё и считается.
    let maxrate = maxrate.max(target_kbps.saturating_add(1));
    (maxrate, maxrate)
}

/// Какой потолок ставить, чтобы настоящий пик не превысил заданного.
///
/// Обратная задача к [`peak_control`]: канал зрителя рассчитан на пик, а не на среднее.
pub fn maxrate_for_peak(peak_kbps: u32) -> u32 {
    peak_kbps.saturating_mul(100) / PEAK_OVER_MAXRATE
}

/// Составить план.
///
/// Возвращает **все** замечания сразу, а не первое: их бывает несколько, и человеку
/// нужно увидеть весь список, а не разбираться по одному за круг.
pub fn plan(
    source: &SourceFile,
    request: &ConvertRequest,
) -> Result<ConvertPlan, Vec<PlanProblem>> {
    let mut problems = Vec::new();

    if source.audio_tracks.is_empty() {
        problems.push(PlanProblem::NoAudioTracks);
    } else if source.track(request.audio_track).is_none() {
        problems.push(PlanProblem::NoSuchTrack {
            index: request.audio_track,
            available: source.audio_tracks.len(),
        });
    }

    if let Some(h) = request.height {
        if h == 0 {
            problems.push(PlanProblem::HeightZero);
        } else if h > source.height {
            // Растягивать нечего: подробностей, которых нет в источнике, не прибавится,
            // а файл раздуется. Это ровно тот случай, о котором FR-029 говорит
            // «не позволять молча».
            problems.push(PlanProblem::HeightAboveSource {
                asked: h,
                source: source.height,
            });
        }
    }

    if let Some(kbps) = request.target_kbps {
        if kbps == 0 {
            problems.push(PlanProblem::BitrateZero);
        } else if u64::from(kbps) * 1000 > source.bitrate_bps.saturating_mul(2) {
            problems.push(PlanProblem::BitrateAboveSource {
                asked_kbps: kbps,
                source_kbps: source.bitrate_bps / 1000,
            });
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }

    let track = source
        .track(request.audio_track)
        .expect("дорожка проверена выше");

    let tonemap = source.is_hdr();
    let downscale = request.height.is_some_and(|h| h != source.height);
    let level = h264_level(
        source.width,
        request.height.unwrap_or(source.height),
        source.fps,
    );

    Ok(ConvertPlan {
        video: video_action(source, request, level, tonemap, downscale),
        audio: audio_action(track),
        audio_track: request.audio_track,
        // Опорный кадр раз в секунду при любой частоте кадров.
        gop: source.fps.max(1),
        tonemap,
        requested_height: request.height,
        faststart: true,
    })
}

fn video_action(
    source: &SourceFile,
    request: &ConvertRequest,
    level: &str,
    tonemap: bool,
    downscale: bool,
) -> VideoAction {
    // Перенос без пересжатия возможен, только когда трогать поток не надо вовсе:
    // любое изменение картинки требует её раскодировать, а раскодировав, обратно
    // «как было» уже не сложить.
    let reason = if !source.video_codec.eq_ignore_ascii_case("h264") {
        Some(Detail::new(DetailCode::ReasonVideoNotH264).with("codec", source.video_codec.clone()))
    } else if !source.pix_fmt.eq_ignore_ascii_case("yuv420p") {
        // Ten-bit H.264 is formally the same codec, but a strict decoder refuses it.
        Some(Detail::new(DetailCode::ReasonVideoPixFmt).with("pix_fmt", source.pix_fmt.clone()))
    } else if tonemap {
        Some(Detail::new(DetailCode::ReasonTonemap))
    } else if downscale {
        Some(Detail::new(DetailCode::ReasonResize))
    } else {
        None
    };

    match (reason, request.target_kbps) {
        (None, None) => VideoAction::Copy,
        // Битрейт задан — пересжимать придётся, даже если поток совместим: иначе
        // требование останется невыполненным, а человек будет думать, что оно учтено.
        (reason, Some(kbps)) => {
            let (maxrate_kbps, bufsize_kbps) = peak_control(kbps);
            VideoAction::ReencodeCapped {
                reason: reason.unwrap_or_else(|| Detail::new(DetailCode::ReasonTargetBitrate)),
                level: level.to_owned(),
                target_kbps: kbps,
                maxrate_kbps,
                bufsize_kbps,
            }
        }
        (Some(reason), None) => VideoAction::Reencode {
            reason,
            level: level.to_owned(),
        },
    }
}

fn audio_action(track: &AudioTrack) -> AudioAction {
    // Три условия обязательны, и это записанная ошибка: проверка одного лишь кодека
    // пропускала шестиканальную дорожку вопреки целевому формату — на входе AAC 5.1
    // файл уезжал шестиканальным.
    let подходит_кодек = track.codec.eq_ignore_ascii_case("aac");
    let стерео = track.channels == 2;
    let бюджет = u64::from(AUDIO_KBPS) * 1000 * (100 + AUDIO_TOLERANCE_PERCENT) / 100;
    let не_толще = track.bitrate_bps.is_none_or(|b| b <= бюджет);

    if подходит_кодек && стерео && не_толще {
        return AudioAction::Copy;
    }

    let reason = if !подходит_кодек {
        Detail::new(DetailCode::ReasonAudioNotAac).with("codec", track.codec.clone())
    } else if !стерео {
        Detail::new(DetailCode::ReasonAudioChannels).with("channels", track.channels)
    } else {
        Detail::new(DetailCode::ReasonAudioTooFat)
    };

    AudioAction::Reencode {
        reason,
        bitrate_kbps: AUDIO_KBPS,
        resample_fix: true,
    }
}
