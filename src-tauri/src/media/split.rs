//! T457, T458, T459 — cutting a film into pieces and putting them back together.
//!
//! **Nothing is re-encoded here, in either direction.** The pieces go to a 3D converter that
//! is not part of this application; re-encoding on the way out would mean the converter sees
//! second-generation material, and re-encoding on the way back would put a third generation on
//! top of its work. Both directions copy.
//!
//! **The audio never goes.** A converter is handed video alone, and the original track is put
//! back on the joined result. Sending the audio through would re-encode it once per piece and
//! leave a seam at every join — and there is nothing a 3D converter can do to a soundtrack
//! that would justify either.

use std::path::Path;

/// What one piece is: where it starts and what it is called.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub index: usize,
    pub starts_at_s: f64,
    pub file: String,
}

/// The arguments that cut a film into pieces at the given times.
///
/// **`-c copy` and nothing else**: the pieces are the source's own bits. `-an` keeps the audio
/// out (T458). `-reset_timestamps 1` starts every piece at nought — without it a converter
/// reading the third piece sees timestamps beginning at two hundred seconds, which some tools
/// take as a stream that starts late and pad with nothing.
///
/// **`-map 0:v:0` and not `-map 0`**: a file with two video streams — a cover image is one —
/// would otherwise put both into every piece, and the cover would be re-encoded into a
/// slideshow. Only the first video stream goes.
pub fn cut_args(source: &Path, at_s: &[f64], pattern: &str) -> Vec<String> {
    let times = at_s
        .iter()
        .map(|t| format!("{t:.3}"))
        .collect::<Vec<_>>()
        .join(",");
    vec![
        String::from("-hide_banner"),
        String::from("-loglevel"),
        String::from("error"),
        String::from("-y"),
        String::from("-i"),
        source.to_string_lossy().into_owned(),
        String::from("-map"),
        String::from("0:v:0"),
        String::from("-an"),
        String::from("-c"),
        String::from("copy"),
        String::from("-f"),
        String::from("segment"),
        String::from("-segment_times"),
        times,
        String::from("-reset_timestamps"),
        String::from("1"),
        pattern.to_owned(),
    ]
}

/// What a piece has to match for the joining to be lossless.
///
/// **The comparison is the whole of T459.** `concat` will happily join pieces that differ and
/// produce a file that opens, plays, and falls apart at the seam — halfway through, on
/// somebody else's screen, hours after anybody could connect it to this. Refusing beforehand
/// costs a person one message; not refusing costs them the evening they were watching.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    /// As ffprobe writes it: `24/1`, `30000/1001`.
    pub frame_rate: String,
    /// The time base, likewise: `1/12288`.
    pub time_base: String,
}

/// Which field of a piece differs from the first one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Differs {
    Codec,
    Frame,
    PixelFormat,
    FrameRate,
    TimeBase,
}

/// Compare a piece with the one the join will start from.
///
/// Returns `None` when they match. The first piece is the reference rather than the source:
/// what comes back from the converter is what is being joined, and if all of them changed in
/// the same way that is a converted film, not a broken one.
pub fn differs(first: &Shape, other: &Shape) -> Option<Differs> {
    if !first.codec.eq_ignore_ascii_case(&other.codec) {
        return Some(Differs::Codec);
    }
    if first.width != other.width || first.height != other.height {
        return Some(Differs::Frame);
    }
    if first.pix_fmt != other.pix_fmt {
        return Some(Differs::PixelFormat);
    }
    if first.frame_rate != other.frame_rate {
        return Some(Differs::FrameRate);
    }
    // **The time base is in here on purpose**, and it is the one a person would never think
    // to check. Two pieces at the same frame rate with different time bases join into a file
    // whose timestamps drift, and the drift shows up as audio sliding out of step — minutes
    // in, long after the seam.
    if first.time_base != other.time_base {
        return Some(Differs::TimeBase);
    }
    None
}

/// The list `concat` reads, in the order the pieces go back together.
///
/// Quoted the way the demuxer wants: a single quote inside a name is written `'\''`. A path
/// with an apostrophe in it is not exotic — a film called `Assassin's Creed` is enough — and
/// getting this wrong makes a list that reads as a different file, or as none.
pub fn concat_list(pieces: &[String]) -> String {
    let mut out = String::new();
    for piece in pieces {
        out.push_str("file '");
        out.push_str(&piece.replace('\'', "'\\''"));
        out.push_str("'\n");
    }
    out
}

/// The arguments that join the pieces and put the original audio back.
///
/// **Two inputs**: the list of pieces and the film the audio came from. The video is copied
/// from the join, the audio from the original, and `-shortest` is deliberately absent — the
/// joined video is the length that matters, and cutting it to the audio would hide a piece
/// that came back short instead of showing it.
pub fn join_args(list: &Path, audio_from: &Path, out: &Path) -> Vec<String> {
    vec![
        String::from("-hide_banner"),
        String::from("-loglevel"),
        String::from("error"),
        String::from("-y"),
        String::from("-f"),
        String::from("concat"),
        // The list names files by path, and without this ffmpeg refuses to read any that are
        // not beside it. The pieces are ours and the list is ours; the risk this guards
        // against is a list from somewhere else, and there is no such list here.
        String::from("-safe"),
        String::from("0"),
        String::from("-i"),
        list.to_string_lossy().into_owned(),
        String::from("-i"),
        audio_from.to_string_lossy().into_owned(),
        String::from("-map"),
        String::from("0:v:0"),
        String::from("-map"),
        String::from("1:a?"),
        String::from("-c"),
        String::from("copy"),
        // MP4 wants its index at the front, or a player has to read the whole file before it
        // can start. The same reason it is on every file this application makes.
        String::from("-movflags"),
        String::from("+faststart"),
        out.to_string_lossy().into_owned(),
    ]
}
