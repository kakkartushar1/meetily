//! Tauri commands for the NeMo ASR engine.
//!
//! These commands mirror the existing parakeet_engine::commands pattern
//! for consistency across the frontend API.

use crate::nemo_engine::nemo_engine::NemoEngine;
use log::{error, info, warn};
use std::sync::{Arc, Mutex};
use tauri::command;
use tauri::{AppHandle, Emitter, Manager, Runtime};

// ============================================================================
// GLOBAL ENGINE STATE
// ============================================================================

pub static NEMO_ENGINE: std::sync::LazyLock<Mutex<Option<Arc<NemoEngine>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

// ============================================================================
// INITIALIZATION
// ============================================================================

/// Initialize the NeMo engine with the app's models directory.
#[command]
pub async fn nemo_init<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let mut guard = NEMO_ENGINE.lock().map_err(|e| format!("Lock error: {}", e))?;

    if guard.is_some() {
        return Ok("NeMo engine already initialized".to_string());
    }

    // Use the same models root as other engines
    let models_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.join("models"))
        .ok();

    let engine = NemoEngine::new(models_dir)
        .map_err(|e| format!("Failed to create NeMo engine: {}", e))?;

    *guard = Some(Arc::new(engine));
    info!("NeMo engine initialized");
    Ok("NeMo engine initialized".to_string())
}

// ============================================================================
// MODEL MANAGEMENT COMMANDS
// ============================================================================

/// Get available NeMo models.
#[command]
pub async fn nemo_get_available_models() -> Result<Vec<super::NemoModelInfo>, String> {
    let engine = get_engine()?;
    engine
        .get_available_models()
        .await
        .map_err(|e| format!("Failed to get NeMo models: {}", e))
}

/// Download a NeMo model.
#[command]
pub async fn nemo_download_model<R: Runtime>(
    app: AppHandle<R>,
    model_id: String,
) -> Result<(), String> {
    let engine = get_engine()?;

    // Emit download start event
    let _ = app.emit(
        "nemo-model-download-progress",
        serde_json::json!({
            "modelName": &model_id,
            "progress": 0
        }),
    );

    match engine.download_model(&model_id).await {
        Ok(()) => {
            let _ = app.emit(
                "nemo-model-download-complete",
                serde_json::json!({
                    "modelName": &model_id
                }),
            );
            info!("NeMo model download complete: {}", model_id);
            Ok(())
        }
        Err(e) => {
            let error_msg = e.to_string();
            let _ = app.emit(
                "nemo-model-download-error",
                serde_json::json!({
                    "modelName": &model_id,
                    "error": &error_msg
                }),
            );
            error!("NeMo model download failed: {}", error_msg);
            Err(error_msg)
        }
    }
}

/// Cancel a NeMo model download.
#[command]
pub async fn nemo_cancel_download(model_id: String) -> Result<(), String> {
    let engine = get_engine()?;
    engine
        .cancel_download(&model_id)
        .await
        .map_err(|e| format!("Failed to cancel download: {}", e))
}

/// Load a NeMo model.
#[command]
pub async fn nemo_load_model(model_id: String) -> Result<(), String> {
    let engine = get_engine()?;
    engine
        .load_model(&model_id)
        .await
        .map_err(|e| format!("Failed to load NeMo model: {}", e))
}

/// Transcribe audio using the loaded NeMo model.
#[command]
pub async fn nemo_transcribe_audio(audio_data: Vec<f32>) -> Result<String, String> {
    let engine = get_engine()?;
    engine
        .transcribe_audio(audio_data)
        .await
        .map_err(|e| format!("NeMo transcription failed: {}", e))
}

/// Validate that a NeMo model is ready.
#[command]
pub async fn nemo_validate_model_ready(model_id: String) -> Result<String, String> {
    let engine = get_engine()?;
    engine
        .validate_model_ready(&model_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(model_id)
}

/// Validate NeMo model ready with config (matches parakeet pattern).
pub async fn nemo_validate_model_ready_with_config<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<String, String> {
    let engine = get_engine()?;

    // Get configured model from transcript settings
    let model_id = match crate::api::api::api_get_transcript_config(
        app.clone(),
        app.clone().state(),
        None,
    )
    .await
    {
        Ok(Some(config)) if config.provider == "parakeet" => {
            if crate::model_catalog::is_nemo_model(&config.model) {
                config.model
            } else {
                return Err("Configured model is not a NeMo model".to_string());
            }
        }
        _ => {
            return Err("No NeMo model configured".to_string());
        }
    };

    // Validate model files exist
    engine
        .validate_model_ready(&model_id)
        .await
        .map_err(|e| e.to_string())?;

    // Ensure sidecar is running and model is loaded
    engine
        .ensure_sidecar_running()
        .await
        .map_err(|e| format!("Failed to start NeMo sidecar: {}", e))?;

    engine
        .load_model(&model_id)
        .await
        .map_err(|e| format!("Failed to load NeMo model: {}", e))?;

    Ok(model_id)
}

/// Open the NeMo models folder in the system file manager.
#[command]
pub async fn open_nemo_models_folder() -> Result<(), String> {
    let engine = get_engine()?;
    let dir = engine.get_models_directory();

    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create models directory: {}", e))?;
    }

    let folder_path = dir.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    info!("Opened NeMo models folder: {}", folder_path);
    Ok(())
}

/// Delete a NeMo model.
#[command]
pub async fn nemo_delete_model(model_id: String) -> Result<String, String> {
    let engine = get_engine()?;
    engine
        .delete_model(&model_id)
        .await
        .map_err(|e| format!("Failed to delete model: {}", e))?;
    Ok(format!("Deleted NeMo model: {}", model_id))
}

/// Unload the current NeMo model.
#[command]
pub async fn nemo_unload_model() -> Result<(), String> {
    let engine = get_engine()?;
    engine
        .unload_model()
        .await
        .map_err(|e| format!("Failed to unload model: {}", e))
}

// ============================================================================
// HELPERS
// ============================================================================

/// Get the NeMo engine instance.
fn get_engine() -> Result<Arc<NemoEngine>, String> {
    let guard = NEMO_ENGINE
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "NeMo engine not initialized. Call nemo_init first.".to_string())
}
