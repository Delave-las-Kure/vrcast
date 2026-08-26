//! What each encoder's options are actually called.
//!
//! **Their dialects are not interchangeable, and assuming they are fails outright.** Asked
//! of the bundled build on 2026-08-26: `h264_nvenc` has `-cq`, `-tune`, `-multipass`,
//! `-rc-lookahead` and `-spatial-aq`; `h264_amf` has none of them and pins quality with
//! `-rc cqp -qp_i -qp_p -qp_b`; `h264_qsv` uses `-global_quality`. An option an encoder does
//! not know is not ignored — ffmpeg refuses to start.
//!
//! One module because there are two callers — preparing a file and probing the material —
//! and two copies of a dialect would drift. The drift would show up as an encode that works
//! on one path and refuses on the other, on somebody else's machine.
//!
//! **⚠ The numbers are calibrated for NVENC only.** The settings carried over from
//! `plan-ladder.sh` and `convert.sh` were measured against it (principle VI). A quantizer of
//! 26 does not mean the same thing to AMD's encoder as to NVIDIA's, and nobody has measured
//! what it does mean. So the other families get equivalent settings — the same intent, in
//! their own words — and everything built on them is marked as resting on an uncalibrated
//! number rather than on a measurement.

use super::encoders::Encoder;

/// Which family an encoder belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// NVIDIA. The one everything here was measured against.
    Nvenc,
    /// AMD.
    Amf,
    /// Intel's, built into the processor.
    Qsv,
    /// The general Linux path, used by both Intel and AMD there.
    Vaapi,
    /// Software x264.
    X264,
    /// Something in the build we have not tried.
    Unknown,
}

pub fn family_of(encoder: &Encoder) -> Family {
    match encoder {
        Encoder::Software => Family::X264,
        Encoder::Hardware { name } => match name.to_ascii_lowercase().as_str() {
            "h264_nvenc" => Family::Nvenc,
            "h264_amf" => Family::Amf,
            "h264_qsv" => Family::Qsv,
            "h264_vaapi" => Family::Vaapi,
            _ => Family::Unknown,
        },
    }
}

impl Family {
    /// Whether what was measured for this project applies to this family.
    ///
    /// Only NVIDIA's. Everything else gets settings that mean the same thing and land
    /// somewhere unmeasured, and whoever uses the answer has to be told so.
    pub fn is_calibrated(&self) -> bool {
        matches!(self, Family::Nvenc)
    }

    /// Whether an encode can be built for this family at all.
    ///
    /// VAAPI cannot, and not for want of trying: it needs a device opened and the frames
    /// uploaded to it before any encoding happens, and a command without that fails on
    /// start. Better to fall back to the processor, which works, than to hand somebody a
    /// command that cannot run.
    pub fn is_usable(&self) -> bool {
        !matches!(self, Family::Vaapi | Family::Unknown)
    }
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

/// The settings that make the encoder work slowly and well.
///
/// NVIDIA's are the measured ones, carried over from `convert.sh` and `plan-ladder.sh`
/// unchanged. The rest say the same thing in their own dialect.
pub fn quality_preset(family: Family) -> Vec<String> {
    match family {
        Family::Nvenc => owned(&[
            // `-rc vbr` sits here rather than with the quality number because convert.sh
            // passes it on both paths: with `-cq` and with `-b:v`. Left with the quality
            // number alone, the capped path would run in whichever mode the encoder
            // defaulted to.
            "-rc",
            "vbr",
            "-preset",
            "p7",
            "-tune",
            "hq",
            "-multipass",
            "fullres",
            "-rc-lookahead",
            "32",
            "-spatial-aq",
            "1",
            "-temporal-aq",
            "1",
            "-aq-strength",
            "8",
            "-bf",
            "3",
            "-coder",
            "cabac",
        ]),
        // `quality` is AMD's slowest and best; `transcoding` is the usage that is not
        // trying to keep latency down at the cost of the picture.
        Family::Amf => owned(&["-usage", "transcoding", "-quality", "quality"]),
        Family::Qsv => owned(&["-preset", "veryslow"]),
        Family::X264 => owned(&["-preset", "slow"]),
        Family::Vaapi | Family::Unknown => Vec::new(),
    }
}

/// The settings that pin quality and let the bitrate land where it lands.
///
/// This is what makes a probe a measurement: nothing is asked of the bitrate, so what comes
/// out is what the material wanted.
pub fn quality_pinned(family: Family, quality: u32) -> Vec<String> {
    match family {
        Family::Nvenc => owned(&["-cq", &quality.to_string()]),
        // AMD has no equivalent of `-cq`: constant quantiser is its own rate-control mode,
        // and the quantiser is given per frame kind.
        Family::Amf => {
            let q = quality.to_string();
            owned(&["-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-qp_b", &q])
        }
        // Intel's is a general option rather than a private one, which is why it does not
        // show up in the encoder's own list.
        Family::Qsv => owned(&["-global_quality", &quality.to_string()]),
        Family::X264 => owned(&["-crf", &quality.to_string()]),
        Family::Vaapi | Family::Unknown => Vec::new(),
    }
}

/// The settings that hold a bitrate to a target with a ceiling and a buffer.
///
/// The same three numbers in every dialect: these are general options rather than any
/// encoder's own, and that is why the capped path has worked on all of them all along.
pub fn bitrate_capped(target_kbps: u32, maxrate_kbps: u32, bufsize_kbps: u32) -> Vec<String> {
    vec![
        String::from("-b:v"),
        format!("{target_kbps}k"),
        String::from("-maxrate"),
        format!("{maxrate_kbps}k"),
        // The buffer equals the ceiling on purpose. A larger one lets bursts run above the
        // ceiling, and that is what froze viewers: a ceiling of 45 with a buffer of 60
        // produced peaks of 54.
        String::from("-bufsize"),
        format!("{bufsize_kbps}k"),
    ]
}
