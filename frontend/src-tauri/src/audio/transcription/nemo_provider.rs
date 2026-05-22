// audio/transcription/nemo_provider.rs
//
// NeMo transcription provider implementation.
// Routes transcription requests through the NeMo Python sidecar.

use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use log::warn;
use std::sync::Arc;

/// NeMo transcription provider (wraps NemoEngine sidecar)
pub struct NemoProvider {
    engine: Arc<crate::nemo_engine::NemoEngine>,
}

impl NemoProvider {
    pub fn new(engine: Arc<crate::nemo_engine::NemoEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl TranscriptionProvider for NemoProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        // Log language preference warning if set (NeMo RNNT is English-only)
        if let Some(ref lang) = language {
            if lang != "en" && lang != "auto" && lang != "auto-translate" {
                warn!(
                    "NeMo RNNT model is English-only; ignoring language preference '{}'",
                    lang
                );
            }
        }

        match self.engine.transcribe_audio(audio).await {
            Ok(text) => Ok(TranscriptResult {
                text: text.trim().to_string(),
                confidence: None, // NeMo doesn't provide confidence scores via this API
                is_partial: false,
            }),
            Err(e) => Err(TranscriptionError::EngineFailed(e.to_string())),
        }
    }

    async fn is_model_loaded(&self) -> bool {
        self.engine.is_model_loaded().await
    }

    async fn get_current_model(&self) -> Option<String> {
        self.engine.get_current_model().await
    }

    fn provider_name(&self) -> &'static str {
        "NeMo"
    }
}
