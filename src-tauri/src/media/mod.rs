//! Working with video files: the bundled FFmpeg, examining a source, preparing it.
//!
//! This layer is separated from `domain` by the same rule as the others: everything
//! that needs a disk and other people's programs lives here, and the decisions live a
//! floor below in `domain`, where they can be tested without a single file on disk.

pub mod convert;
pub mod encoders;
pub mod ffmpeg;
pub mod probe;
pub mod validate;
