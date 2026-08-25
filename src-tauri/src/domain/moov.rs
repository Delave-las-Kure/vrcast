//! T033a — parsing the `moov` atom: a video's parameters straight from the MP4 header
//! (R-19, FR-012).
//!
//! Why parse it ourselves instead of using FFmpeg. To show the resolution, the duration and
//! the codecs of a file that lies on the server, a few hundred kilobytes of its beginning
//! have to be read — not the whole file downloaded, and no external program run. For a file
//! prepared by our own process (`-movflags +faststart`) the header is right at the
//! beginning.
//!
//! What this parser does when the header is not at the beginning: **it does not guess**. It
//! answers "unknown" along with the fact that the file does not match the target format
//! (FR-012). A viewer will only start watching such a file after downloading its tail, and
//! knowing that matters more to a person than seeing the resolution.
//!
//! The parsing runs over untrusted data: the file on the server could turn out to be
//! anything. So there is not one indexing without a bounds check here — only functions that
//! return `Option`, and a limit on nesting depth.

/// How deep we are willing to descend through nested atoms.
///
/// The real depth down to the codec — `moov/trak/mdia/minf/stbl/stsd` — is six levels. The
/// limit guards against a file put together so that the parsing descends forever.
const MAX_DEPTH: usize = 8;

/// How many bytes of the header to ask the server for on the first attempt.
///
/// In a file with `moov` at the beginning the header usually fits in a few hundred
/// kilobytes: it grows with the number of frames. Should that not be enough, the parser says
/// exactly how much is needed (see [`MoovOutcome::NeedMoreBytes`]), and a second attempt is
/// certain to do.
pub const SUGGESTED_HEAD_BYTES: u64 = 512 * 1024;

/// A file's parameters, read out of its header.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaParams {
    pub duration_s: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The average bitrate: the file's size divided by its duration. Worked out only when
    /// both are known.
    pub bitrate_bps: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

/// How the parsing ended.
#[derive(Debug, Clone, PartialEq)]
pub enum MoovOutcome {
    /// The header was found at the beginning and parsed.
    Parsed(MediaParams),
    /// The header is there, but it lies after the data: the file is not prepared for
    /// serving. The parameters stay unknown — reading them out would mean downloading the
    /// whole file.
    MoovAfterData,
    /// The piece that was read did not suffice. `need` is how many bytes from the start of
    /// the file are wanted to carry on.
    NeedMoreBytes { need: u64 },
    /// This is not MP4.
    NotMp4,
}

impl MoovOutcome {
    /// The value of a served file's `faststart_ok` field.
    ///
    /// `None` means the question is still open: there was not enough data, or this is not
    /// MP4 at all, and there are no grounds for declaring the file unfit.
    pub fn faststart_ok(&self) -> Option<bool> {
        match self {
            Self::Parsed(_) => Some(true),
            Self::MoovAfterData => Some(false),
            Self::NeedMoreBytes { .. } | Self::NotMp4 => None,
        }
    }

    pub fn params(&self) -> Option<&MediaParams> {
        match self {
            Self::Parsed(p) => Some(p),
            _ => None,
        }
    }
}

/// Parse the beginning of an MP4 file.
///
/// `head` is the bytes read from the start of the file. `file_size` is the full size of the
/// file, when it is known: without it the average bitrate cannot be worked out, but
/// everything else parses.
pub fn parse(head: &[u8], file_size: Option<u64>) -> MoovOutcome {
    let mut offset: usize = 0;
    let mut looks_like_mp4 = false;

    loop {
        let header = match BoxHeader::read(head, offset) {
            Ok(h) => h,
            // There was not enough data even for the header. Asking for more makes sense
            // only if the beginning has already been recognised as MP4.
            Err(HeaderError::Truncated) if looks_like_mp4 => {
                return need_more(offset as u64 + 16, file_size)
            }
            // The header contradicts itself: its length is less than the header. Asking
            // for more will not help — what has been read does not add up as it is.
            Err(_) => return MoovOutcome::NotMp4,
        };

        if !looks_like_mp4 {
            // The first atom must be one of those an MP4 begins with. A single check that
            // the name is made of printable characters is not enough: a server's error page
            // ("<!DOCTYPE html>") has "CTYP" where the name should be, and without this
            // list it would pass for a video with a gigabyte-long atom.
            if !header.starts_a_file() {
                return MoovOutcome::NotMp4;
            }
            looks_like_mp4 = true;
        } else if !header.type_is_plausible() {
            return MoovOutcome::NotMp4;
        }

        match &header.typ {
            b"moov" => {
                let Some(payload) = header.payload(head) else {
                    // The atom began but ends beyond what was read. We know exactly how
                    // much is needed — so we ask for exactly that much.
                    return need_more(offset as u64 + header.total_len, file_size);
                };
                let params = parse_moov(payload, file_size);
                return MoovOutcome::Parsed(params);
            }
            // The data came before the header — the file is not prepared for serving.
            // There is no point looking further: the answer is already known, and `mdat`
            // may run to gigabytes, which it is senseless to read through for a header.
            b"mdat" => return MoovOutcome::MoovAfterData,
            _ => {}
        }

        let Some(next) = header.next_offset(offset) else {
            return MoovOutcome::NotMp4;
        };
        if next <= offset {
            // An atom of zero length: the parsing would never move on.
            return MoovOutcome::NotMp4;
        }
        if next >= head.len() {
            return need_more(next as u64 + 16, file_size);
        }
        offset = next;
    }
}

/// Ask for the missing bytes — but only if the file holds them at all.
///
/// Without this check the parser would ask for data past the end of the file, the reading
/// layer would hand back the same piece, and the parser would ask again: an endless circle
/// on a file that has no header at all. When there is nothing more to ask for, the answer is
/// honest: this is not a video we know how to work with.
fn need_more(need: u64, file_size: Option<u64>) -> MoovOutcome {
    match file_size {
        Some(size) if need > size => MoovOutcome::NotMp4,
        _ => MoovOutcome::NeedMoreBytes { need },
    }
}

/// What is known about one track.
#[derive(Debug, Default)]
struct Track {
    handler: Option<[u8; 4]>,
    codec: Option<String>,
    /// The resolution from the sample description — what the frame is encoded at.
    coded_size: Option<(u32, u32)>,
    /// The resolution from the track header — how it is to be shown.
    display_size: Option<(u32, u32)>,
    timescale: Option<u32>,
    duration: Option<u64>,
}

impl Track {
    fn is_video(&self) -> bool {
        self.handler.as_ref().is_some_and(|h| h == b"vide")
    }

    fn is_audio(&self) -> bool {
        self.handler.as_ref().is_some_and(|h| h == b"soun")
    }

    fn seconds(&self) -> Option<f64> {
        let ts = self.timescale.filter(|t| *t > 0)?;
        let d = self
            .duration
            .filter(|d| *d > 0 && *d != u64::from(u32::MAX))?;
        Some(d as f64 / f64::from(ts))
    }
}

fn parse_moov(payload: &[u8], file_size: Option<u64>) -> MediaParams {
    let mut params = MediaParams::default();
    let mut movie_seconds: Option<f64> = None;
    let mut tracks: Vec<Track> = Vec::new();

    walk(payload, 0, |typ, body| match typ {
        b"mvhd" => {
            movie_seconds = parse_mvhd(body);
        }
        b"trak" => {
            let mut track = Track::default();
            parse_trak(body, &mut track, 1);
            tracks.push(track);
        }
        _ => {}
    });

    let video = tracks.iter().find(|t| t.is_video());
    let audio = tracks.iter().find(|t| t.is_audio());

    if let Some(v) = video {
        // The resolution is taken from the sample description: that is what the frame is
        // encoded at, and it is what the rungs of a quality ladder are compared against.
        // The track header holds the size for display, which differs on anamorphic video.
        let (w, h) = v.coded_size.or(v.display_size).unwrap_or((0, 0));
        if w > 0 && h > 0 {
            params.width = Some(w);
            params.height = Some(h);
        }
        params.video_codec = v.codec.clone();
    }
    if let Some(a) = audio {
        params.audio_codec = a.codec.clone();
    }

    // The movie's duration comes from the movie header; when it is not there (an
    // "unknown" does turn up), the longest track is taken instead.
    params.duration_s = movie_seconds.or_else(|| {
        tracks
            .iter()
            .filter_map(Track::seconds)
            .fold(None, |acc: Option<f64>, s| {
                Some(acc.map_or(s, |a| a.max(s)))
            })
    });

    if let (Some(size), Some(seconds)) = (file_size, params.duration_s) {
        if seconds > 0.0 && size > 0 {
            let bps = (size as f64 * 8.0 / seconds).round();
            if bps.is_finite() && bps >= 0.0 {
                params.bitrate_bps = Some(bps as u64);
            }
        }
    }

    params
}

fn parse_mvhd(body: &[u8]) -> Option<f64> {
    let version = *body.first()?;
    let (timescale, duration) = if version == 1 {
        (be_u32(body, 20)?, be_u64(body, 24)?)
    } else {
        (be_u32(body, 12)?, u64::from(be_u32(body, 16)?))
    };
    if timescale == 0 || duration == 0 || duration == u64::from(u32::MAX) || duration == u64::MAX {
        return None;
    }
    Some(duration as f64 / f64::from(timescale))
}

fn parse_trak(body: &[u8], track: &mut Track, depth: usize) {
    walk(body, depth, |typ, inner| match typ {
        b"tkhd" => {
            track.display_size = parse_tkhd_size(inner);
        }
        b"mdia" => {
            walk(inner, depth + 1, |t, b| match t {
                b"mdhd" => {
                    if let Some((ts, d)) = parse_mdhd(b) {
                        track.timescale = Some(ts);
                        track.duration = Some(d);
                    }
                }
                b"hdlr" => {
                    track.handler = parse_hdlr(b);
                }
                b"minf" => {
                    walk(b, depth + 2, |t2, b2| {
                        if t2 == b"stbl" {
                            walk(b2, depth + 3, |t3, b3| {
                                if t3 == b"stsd" {
                                    if let Some((codec, size)) = parse_stsd(b3) {
                                        track.codec = Some(codec);
                                        track.coded_size = size;
                                    }
                                }
                            });
                        }
                    });
                }
                _ => {}
            });
        }
        _ => {}
    });
}

fn parse_tkhd_size(body: &[u8]) -> Option<(u32, u32)> {
    let version = *body.first()?;
    // The fields before the transformation matrix: in version zero the times and the
    // duration are four bytes each, in version one they are eight.
    let base = if version == 1 { 4 + 32 } else { 4 + 20 };
    let after_matrix = base + 8 + 2 + 2 + 2 + 2 + 36;
    let width = be_u32(body, after_matrix)?;
    let height = be_u32(body, after_matrix + 4)?;
    // The values are written with their fractional part in the low sixteen bits.
    let (w, h) = (width >> 16, height >> 16);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

fn parse_mdhd(body: &[u8]) -> Option<(u32, u64)> {
    let version = *body.first()?;
    if version == 1 {
        Some((be_u32(body, 20)?, be_u64(body, 24)?))
    } else {
        Some((be_u32(body, 12)?, u64::from(be_u32(body, 16)?)))
    }
}

fn parse_hdlr(body: &[u8]) -> Option<[u8; 4]> {
    let slice = body.get(8..12)?;
    let mut out = [0u8; 4];
    out.copy_from_slice(slice);
    Some(out)
}

/// The codec and, for video, the coded resolution — from the first sample description.
fn parse_stsd(body: &[u8]) -> Option<(String, Option<(u32, u32)>)> {
    // version+flags (4), entry_count (4), and then the first entry.
    let entry = 8usize;
    let entry_size = be_u32(body, entry)?;
    if entry_size < 8 {
        return None;
    }
    let format = body.get(entry + 4..entry + 8)?;
    let codec = codec_name(format);

    // Inside a video entry: reserved(6) + data_reference_index(2) + pre_defined(2) +
    // reserved(2) + pre_defined(12) = 24 bytes before the sizes.
    let size_at = entry + 8 + 24;
    let coded = match (be_u16(body, size_at), be_u16(body, size_at + 2)) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((u32::from(w), u32::from(h))),
        _ => None,
    };

    Some((codec, coded))
}

/// A human name for a codec, from the four-letter format code.
///
/// An unfamiliar code is returned as it stands: showing a person "hvc1" is more honest than
/// saying nothing — they can at least search for what it is.
fn codec_name(format: &[u8]) -> String {
    match format {
        b"avc1" | b"avc3" => String::from("h264"),
        b"hev1" | b"hvc1" => String::from("h265"),
        b"av01" => String::from("av1"),
        b"vp09" => String::from("vp9"),
        b"mp4a" => String::from("aac"),
        b"ac-3" => String::from("ac3"),
        b"ec-3" => String::from("eac3"),
        b"Opus" | b"opus" => String::from("opus"),
        b"fLaC" => String::from("flac"),
        b".mp3" | b"mp3 " => String::from("mp3"),
        other => String::from_utf8_lossy(other).trim().to_owned(),
    }
}

/// Walk the nested atoms, calling the handler for each one.
///
/// It stops quietly at the first place it cannot make sense of: a corrupted tail must not
/// cancel what has already been read from the beginning.
fn walk<F>(data: &[u8], depth: usize, mut visit: F)
where
    F: FnMut(&[u8; 4], &[u8]),
{
    if depth > MAX_DEPTH {
        return;
    }
    let mut offset = 0usize;
    while let Ok(header) = BoxHeader::read(data, offset) {
        if !header.type_is_plausible() {
            return;
        }
        if let Some(payload) = header.payload(data) {
            visit(&header.typ, payload);
        }
        let Some(next) = header.next_offset(offset) else {
            return;
        };
        if next <= offset || next >= data.len() {
            return;
        }
        offset = next;
    }
}

/// Why an atom's header could not be read.
///
/// The difference between the two cases decides whether to ask the server for more data: it
/// helps a truncated piece, and never helps a contradictory one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderError {
    /// There were not enough bytes even for the header.
    Truncated,
    /// The header does not add up: its length is less than its own size.
    Malformed,
}

/// The names of the atoms an MP4 file may begin with.
///
/// By the standard `ftyp` must come first, but files without it do turn up, so the list is a
/// little wider — by exactly what really does appear at the beginning.
const FILE_STARTERS: [&[u8; 4]; 6] = [b"ftyp", b"styp", b"moov", b"free", b"skip", b"mdat"];

/// An atom's header: the name and the length, the header included.
struct BoxHeader {
    typ: [u8; 4],
    /// The header's length: eight bytes usually, sixteen when the length is eight bytes.
    header_len: usize,
    /// The atom's full length, the header included.
    total_len: u64,
    /// The offset of the atom's start within the source data.
    start: usize,
}

impl BoxHeader {
    fn read(data: &[u8], offset: usize) -> Result<Self, HeaderError> {
        let size32 = be_u32(data, offset).ok_or(HeaderError::Truncated)?;
        let mut typ = [0u8; 4];
        typ.copy_from_slice(
            data.get(offset + 4..offset + 8)
                .ok_or(HeaderError::Truncated)?,
        );

        let (header_len, total_len) = match size32 {
            // A one means the real length is written next, in eight bytes.
            1 => (
                16usize,
                be_u64(data, offset + 8).ok_or(HeaderError::Truncated)?,
            ),
            // A zero means "to the end of the file".
            0 => (8usize, (data.len() - offset) as u64),
            n if n < 8 => return Err(HeaderError::Malformed),
            n => (8usize, u64::from(n)),
        };

        if total_len < header_len as u64 {
            return Err(HeaderError::Malformed);
        }

        Ok(Self {
            typ,
            header_len,
            total_len,
            start: offset,
        })
    }

    /// Whether an atom with such a name may stand at the beginning of a file.
    fn starts_a_file(&self) -> bool {
        FILE_STARTERS.contains(&&self.typ)
    }

    /// Whether an atom's name looks like a name rather than like random bytes.
    fn type_is_plausible(&self) -> bool {
        self.typ.iter().all(
            |b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b' ' | b'-' | b'.' | 0xA9),
        )
    }

    /// An atom's contents, if all of it is inside the piece that was read.
    fn payload<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        let from = self.start.checked_add(self.header_len)?;
        let to = usize::try_from(self.total_len)
            .ok()
            .and_then(|len| self.start.checked_add(len))?;
        data.get(from..to)
    }

    fn next_offset(&self, offset: usize) -> Option<usize> {
        usize::try_from(self.total_len)
            .ok()
            .and_then(|len| offset.checked_add(len))
    }
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn be_u64(data: &[u8], at: usize) -> Option<u64> {
    let bytes = data.get(at..at + 8)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(bytes);
    Some(u64::from_be_bytes(out))
}
