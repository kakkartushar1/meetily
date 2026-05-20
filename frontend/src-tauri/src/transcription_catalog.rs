//! Shared transcription model catalog.
//!
//! The saved transcript setting remains `{ provider, model }`; this catalog
//! resolves the model id to the runtime that should execute it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionRuntime {
    WhisperCpp,
    ParakeetOnnx,
    Nemo,
}

#[derive(Debug, Clone, Copy)]
pub struct TranscriptionModelCatalogEntry {
    pub provider: &'static str,
    pub model_id: &'static str,
    pub display_name: &'static str,
    pub runtime: TranscriptionRuntime,
    pub size_mb: u32,
    pub accuracy: &'static str,
    pub speed: &'static str,
    pub description: &'static str,
    pub repo_id: Option<&'static str>,
    pub filename: Option<&'static str>,
}

pub const NEMO_PARAKEET_RNNT_1_1B: &str = "nvidia/parakeet-rnnt-1.1b";

pub const PARAKEET_ONNX_CATALOG: &[TranscriptionModelCatalogEntry] = &[
    TranscriptionModelCatalogEntry {
        provider: "parakeet",
        model_id: "parakeet-tdt-0.6b-v3-int8",
        display_name: "Parakeet TDT 0.6B v3 Int8",
        runtime: TranscriptionRuntime::ParakeetOnnx,
        size_mb: 670,
        accuracy: "High",
        speed: "Ultra Fast (v3)",
        description: "Real time on M4 Max, latest version with int8 quantization",
        repo_id: None,
        filename: None,
    },
    TranscriptionModelCatalogEntry {
        provider: "parakeet",
        model_id: "parakeet-tdt-0.6b-v2-int8",
        display_name: "Parakeet TDT 0.6B v2 Int8",
        runtime: TranscriptionRuntime::ParakeetOnnx,
        size_mb: 661,
        accuracy: "High",
        speed: "Fast (v2)",
        description: "Previous version with int8 quantization, good balance of speed and accuracy",
        repo_id: None,
        filename: None,
    },
];

pub const NEMO_MODEL_CATALOG: &[TranscriptionModelCatalogEntry] = &[
    TranscriptionModelCatalogEntry {
        provider: "parakeet",
        model_id: NEMO_PARAKEET_RNNT_1_1B,
        display_name: "Parakeet RNNT 1.1B",
        runtime: TranscriptionRuntime::Nemo,
        size_mb: 4280,
        accuracy: "High",
        speed: "Medium",
        description: "NVIDIA NeMo RNNT checkpoint for high-accuracy English transcription",
        repo_id: Some("nvidia/parakeet-rnnt-1.1b"),
        filename: Some("parakeet-rnnt-1.1b.nemo"),
    },
];

pub fn get_transcription_model(model_id: &str) -> Option<&'static TranscriptionModelCatalogEntry> {
    PARAKEET_ONNX_CATALOG
        .iter()
        .chain(NEMO_MODEL_CATALOG.iter())
        .find(|entry| entry.model_id == model_id)
}

pub fn parakeet_onnx_models() -> &'static [TranscriptionModelCatalogEntry] {
    PARAKEET_ONNX_CATALOG
}

pub fn nemo_models() -> &'static [TranscriptionModelCatalogEntry] {
    NEMO_MODEL_CATALOG
}

pub fn is_nemo_model(model_id: &str) -> bool {
    get_transcription_model(model_id)
        .map(|entry| entry.runtime == TranscriptionRuntime::Nemo)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_PARAKEET_MODEL;

    #[test]
    fn resolves_default_parakeet_to_onnx_runtime() {
        let entry = get_transcription_model(DEFAULT_PARAKEET_MODEL).unwrap();

        assert_eq!(entry.runtime, TranscriptionRuntime::ParakeetOnnx);
    }

    #[test]
    fn resolves_rnnt_model_to_nemo_runtime() {
        let entry = get_transcription_model(NEMO_PARAKEET_RNNT_1_1B).unwrap();

        assert_eq!(entry.provider, "parakeet");
        assert_eq!(entry.runtime, TranscriptionRuntime::Nemo);
        assert_eq!(entry.repo_id, Some("nvidia/parakeet-rnnt-1.1b"));
        assert_eq!(entry.filename, Some("parakeet-rnnt-1.1b.nemo"));
    }
}
