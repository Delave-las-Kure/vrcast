//! T085 — whether there is room on the server (FR-036).
//!
//! Checked **before** the transfer starts. Learning of a shortage halfway through a
//! thirty-gigabyte upload means losing an hour and leaving a half-transferred tail on
//! the disk.
//!
//! "Enough" is not "fits exactly". A margin is needed for two reasons, both real: for
//! a while the previous version of the same file sits on the disk alongside the staged
//! one (replacement happens by renaming, and the old file disappears only at that
//! instant), and the server also writes logs and must have somewhere to write them. A
//! completely full disk is not "out of space" but a server that stops answering.

use crate::commands::library::DiskUsage;

/// The margin above the file size that must stay free.
///
/// A share of the disk rather than a constant: on a hundred-gigabyte disk five per
/// cent is five gigabytes, which is ample; on a terabyte disk a constant five
/// gigabytes would already be too little.
const RESERVE_FRACTION: f64 = 0.05;

/// The floor for the margin: on a small disk a share comes to too little, and a
/// server stops working properly long before the last byte.
const RESERVE_MIN_BYTES: u64 = 512 * 1024 * 1024;

/// What the space calculation found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceVerdict {
    /// Enough: the file fits and the margin remains.
    Fits,
    /// Not enough. The refusal names how much is missing — otherwise a person has to
    /// work it out themselves.
    NotEnough {
        /// How much is needed in all: the file plus the margin.
        needed: u64,
        /// How much is free now.
        free: u64,
        /// How much is missing.
        short_by: u64,
    },
}

/// Whether there is room for a file of the given size.
///
/// `already_uploaded` is how much is in the staged file already: when a broken
/// transfer resumes, that room is taken, and demanding it again would mean refusing to
/// finish a file that had almost arrived.
pub fn check(disk: &DiskUsage, file_size: u64, already_uploaded: u64) -> SpaceVerdict {
    let reserve = reserve_for(disk.total_bytes);
    let remaining = file_size.saturating_sub(already_uploaded);
    let needed = remaining.saturating_add(reserve);

    if disk.free_bytes >= needed {
        SpaceVerdict::Fits
    } else {
        SpaceVerdict::NotEnough {
            needed,
            free: disk.free_bytes,
            short_by: needed.saturating_sub(disk.free_bytes),
        }
    }
}

/// The size of the margin for a disk of this size.
pub fn reserve_for(total_bytes: u64) -> u64 {
    let fraction = (total_bytes as f64 * RESERVE_FRACTION) as u64;
    fraction.max(RESERVE_MIN_BYTES)
}
