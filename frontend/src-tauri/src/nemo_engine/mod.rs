//! NeMo ASR engine module.
//!
//! Manages a Python NeMo sidecar process for .nemo model inference.
//! The sidecar is lazy-started only when a .nemo model is selected.
//!
//! # Module Structure
//!
//! - `nemo_engine`: Sidecar lifecycle and HTTP client
//! - `commands`: Tauri command interface for frontend integration

pub mod nemo_engine;
pub mod commands;

pub use nemo_engine::{NemoEngine, NemoModelInfo, NemoModelStatus};
