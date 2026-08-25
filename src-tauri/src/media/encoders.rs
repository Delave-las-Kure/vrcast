//! T119 — which encoder to use (FR-026).
//!
//! Three rules, and the third matters most:
//!
//! 1. use the hardware one when it is there;
//! 2. work without it;
//! 3. **do not stay quiet about falling back to the processor**.
//!
//! The third is not politeness. The difference in time is severalfold: what a graphics
//! card does in ten minutes the processor does in an hour and a half. Someone who was
//! not warned decides the application has hung and kills the task halfway.
//!
//! Quality, on the other hand, is nothing to worry about, and that is not a platitude
//! but a measurement taken on 2026-08-02 on our own material: software x264 against
//! NVENC differed by +1.13 on the VMAF scale at four megabits and by nothing (a slight
//! minus, even) at working bitrates of fourteen and above. So the message about the
//! fallback says honestly: you will lose time, not quality.

use crate::domain::wording::{Detail, DetailCode};
use serde::{Deserialize, Serialize};

/// What to encode with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Encoder {
    /// A hardware encoder in a graphics card or a processor.
    Hardware { name: String },
    /// Software x264. Severalfold slower, but at working bitrates it gives up
    /// nothing in quality.
    Software,
}

impl Encoder {
    /// The name FFmpeg understands.
    pub fn ffmpeg_name(&self) -> &str {
        match self {
            Self::Hardware { name } => name,
            Self::Software => "libx264",
        }
    }
}

/// What was chosen, and what to say about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncoderChoice {
    pub encoder: Encoder,
    /// What to say about the choice, if anything. Empty means there is nothing to
    /// warn about: the best available was taken and nobody loses anything.
    pub notice: Option<Detail>,
}

/// Why no choice could be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("nothing to encode with: the bundled build has neither a hardware H.264 encoder nor a software one")]
pub struct NoEncoder;

/// The order of preference.
///
/// NVIDIA first — it is faster than the rest on our material. Then Intel's integrated
/// graphics, then AMD, and last the general Linux path: it works with both Intel and
/// AMD, but there is no sense choosing it when the vendor's own is available.
const PREFERRED: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_vaapi"];

/// Choose an encoder.
///
/// `available` is what the bundled build **knows how to call** (see
/// `ffmpeg::probe_self`). That is not the same as "works on this machine": an encoder
/// being in the build says nothing about the hardware, and only a trial run gives the
/// real answer. So the choice here is a supposition, and testing it is a separate step.
///
/// `prefer_hardware` is false when a person asked for the processor themselves.
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

/// What to say when a hardware encoder let us down in practice.
///
/// Being in the build does not mean working: a graphics card may lack the block, the
/// driver may be old, and on a laptop the card may simply be switched off. Falling
/// back to the processor is the right behaviour in that case, but staying quiet about
/// it is doubly wrong: the person expected ten minutes and will get an hour.
pub fn fallback_notice(failed: &str) -> Detail {
    Detail::new(DetailCode::NoticeHardwareFailed).with("encoder", failed.to_string())
}
