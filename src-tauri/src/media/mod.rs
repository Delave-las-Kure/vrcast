//! Working with video files: the bundled FFmpeg, examining a source, preparing it.
//!
//! This layer is separated from `domain` by the same rule as the others: everything
//! that needs a disk and other people's programs lives here, and the decisions live a
//! floor below in `domain`, where they can be tested without a single file on disk.

pub mod convert;
pub mod encoder_args;
pub mod encoders;
pub mod ffmpeg;
pub mod keyframes;
pub mod local_disk;
pub mod measure;
pub mod probe;
pub mod probe_complexity;
pub mod split;
pub mod validate;
pub mod vmaf;
