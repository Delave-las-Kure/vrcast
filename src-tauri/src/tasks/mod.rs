//! The machinery of long tasks.
//!
//! Preparing a file takes hours, and so does uploading. Through all of it the
//! interface has to stay responsive (FR-080, SC-009), tasks have to survive a restart
//! of the application (FR-081), and cancelling has to end the whole process tree for
//! certain (constitution, principle III).

pub mod engine;
pub mod process;
pub mod progress;
pub mod quality_measure;
pub mod registry;
pub mod state;
pub mod store;
