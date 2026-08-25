//! The error type of the command layer.
//!
//! It lives in [`crate::error`] rather than here: the task engine records the error of
//! a failed task, and a module above the engine could not be seen from inside it.
//! Re-exported so callers of this layer still find it where the contract says it is.

pub use crate::error::*;
