//! T108 — the source file and its tracks (data-model section 6, FR-020, FR-021).
//!
//! Only a description of what was found in the file lives here. None of it decides
//! what to do with the file: that decision lives in [`super::convert_plan`], and the
//! split is deliberate — examining a source happens once, while the plan is worked out
//! again on every movement of a slider.
//!
//! How to caption a track for a person is not here either. It used to be, and nothing
//! but its own tests called it: the interface builds the caption from its catalogue,
//! because the words around the numbers differ between languages while the numbers do
//! not (see `trackLabel` in `ConvertScreen`).

use serde::{Deserialize, Serialize};

/// An audio track of the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// The index among audio tracks, from zero: that is what ffmpeg understands in
    /// `-map 0:a:<N>`. A person is shown it counting from one.
    pub index: usize,
    pub codec: String,
    pub channels: u16,
    /// The track's bitrate, when known.
    pub bitrate_bps: Option<u64>,
    /// The language. Often missing — which is ordinary rather than a fault.
    pub language: Option<String>,
    /// The track title: "Dubbed", "Original", "Director's commentary".
    pub title: Option<String>,
    pub is_default: bool,
}

/// A source file that has been examined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: String,
    pub size_bytes: u64,
    pub duration_s: f64,
    pub width: u32,
    pub height: u32,
    /// Frames per second, rounded **up**: 47.952 is 48-frame material, and rounding
    /// down would understate the compatibility level.
    pub fps: u32,
    pub bitrate_bps: u64,
    /// The peak bitrate, if it was measured. Measuring costs time and is done separately.
    pub peak_bps: Option<u64>,
    pub video_codec: String,
    pub pix_fmt: String,
    /// The colour transfer characteristic. HDR is recognised by it, and HDR has to be
    /// brought down to the ordinary range or a viewer's picture comes out washed out.
    pub color_transfer: Option<String>,
    pub audio_tracks: Vec<AudioTrack>,
}

/// The marks of HDR in a colour transfer characteristic.
const HDR_TRANSFERS: [&str; 4] = ["smpte2084", "arib-std-b67", "smpte428", "bt2020-10"];

impl SourceFile {
    /// The track worth offering by default.
    ///
    /// The one marked as the main one, otherwise the first. Empty only when there is
    /// no audio at all; that is a case of its own rather than "we will take track
    /// zero" (FR-021, code `NO_AUDIO_TRACKS`).
    pub fn default_track(&self) -> Option<&AudioTrack> {
        self.audio_tracks
            .iter()
            .find(|t| t.is_default)
            .or_else(|| self.audio_tracks.first())
    }

    pub fn track(&self, index: usize) -> Option<&AudioTrack> {
        self.audio_tracks.iter().find(|t| t.index == index)
    }

    /// Whether the source was recorded in high dynamic range.
    ///
    /// Not important in itself: such a picture has to be brought down to the ordinary
    /// range, and that means the stream can no longer be copied without re-encoding.
    pub fn is_hdr(&self) -> bool {
        match &self.color_transfer {
            Some(t) => {
                let t = t.to_ascii_lowercase();
                HDR_TRANSFERS.iter().any(|h| t == *h)
            }
            None => false,
        }
    }

    /// How many pixels there are in a frame.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}
