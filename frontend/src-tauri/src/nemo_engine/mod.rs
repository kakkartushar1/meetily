//! NVIDIA NeMo `.nemo` transcription runtime.
//!
//! This module is intentionally separate from the ONNX Parakeet runtime. NeMo
//! checkpoints require Python/PyTorch/NeMo, while existing Parakeet models stay
//! on the Rust ONNX path.

pub mod commands;
pub mod nemo_engine;

pub use nemo_engine::NemoEngine;
