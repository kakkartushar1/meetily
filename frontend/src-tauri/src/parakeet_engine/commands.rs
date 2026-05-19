use crate::parakeet_engine::{ModelInfo, ModelStatus, ParakeetEngine, DownloadProgress};
use crate::parakeet_engine::huggingface_api;
use crate::model_catalog::{
    CustomModelCatalogEntry, CustomModelStatus, ModelFormat,
    detect_model_format, register_custom_model, unregister_custom_model,
    update_custom_model_status, get_custom_models, lookup_custom_model,
};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::Arc;
use tauri::{command, Emitter, AppHandle, Manager, Runtime};

// Global parakeet engine
pub static PARAKEET_ENGINE: Mutex<Option<Arc<ParakeetEngine>>> = Mutex::new(None);

// Global models directory path (set during app initialization)
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Initialize the models directory path using app_data_dir
/// This should be called during app setup before parakeet_init
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = app.path().app_data_dir()
        .expect("Failed to get app data dir");

    let models_dir = app_data_dir.join("models");

    // Create directory if it doesn't exist
    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            log::error!("Failed to create models directory: {}", e);
            return;
        }
    }

    log::info!("Parakeet models directory set to: {}", models_dir.display());

    let mut guard = MODELS_DIR.lock().unwrap();
    *guard = Some(models_dir);
}

/// Get the configured models directory
fn get_models_directory() -> Option<PathBuf> {
    MODELS_DIR.lock().unwrap().clone()
}

#[command]
pub async fn parakeet_init() -> Result<(), String> {
    let mut guard = PARAKEET_ENGINE.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    let models_dir = get_models_directory();
    let engine = ParakeetEngine::new_with_models_dir(models_dir)
        .map_err(|e| format!("Failed to initialize Parakeet engine: {}", e))?;
    *guard = Some(Arc::new(engine));
    Ok(())
}

#[command]
pub async fn parakeet_get_available_models() -> Result<Vec<ModelInfo>, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        engine
            .discover_models()
            .await
            .map_err(|e| format!("Failed to discover Parakeet models: {}", e))
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_load_model<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String
) -> Result<(), String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // Emit model loading started event
        if let Err(e) = app_handle.emit(
            "parakeet-model-loading-started",
            serde_json::json!({
                "modelName": model_name
            }),
        ) {
            log::error!("Failed to emit parakeet-model-loading-started event: {}", e);
        }

        let result = engine
            .load_model(&model_name)
            .await
            .map_err(|e| format!("Failed to load Parakeet model: {}", e));

        // Emit model loading completed/failed event
        if result.is_ok() {
            if let Err(e) = app_handle.emit(
                "parakeet-model-loading-completed",
                serde_json::json!({
                    "modelName": model_name
                }),
            ) {
                log::error!("Failed to emit parakeet-model-loading-completed event: {}", e);
            }
        } else if let Err(ref error) = result {
            if let Err(e) = app_handle.emit(
                "parakeet-model-loading-failed",
                serde_json::json!({
                    "modelName": model_name,
                    "error": error
                }),
            ) {
                log::error!("Failed to emit parakeet-model-loading-failed event: {}", e);
            }
        }

        result
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_get_current_model() -> Result<Option<String>, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        Ok(engine.get_current_model().await)
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_is_model_loaded() -> Result<bool, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        Ok(engine.is_model_loaded().await)
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_has_available_models() -> Result<bool, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        let models = engine
            .discover_models()
            .await
            .map_err(|e| format!("Failed to discover Parakeet models: {}", e))?;

        // Check if at least one model is available
        let available_models: Vec<_> = models
            .iter()
            .filter(|model| matches!(model.status, crate::parakeet_engine::ModelStatus::Available))
            .collect();

        Ok(!available_models.is_empty())
    } else {
        Ok(false)
    }
}

#[command]
pub async fn parakeet_validate_model_ready() -> Result<String, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // Check if a model is currently loaded
        if engine.is_model_loaded().await {
            if let Some(current_model) = engine.get_current_model().await {
                return Ok(current_model);
            }
        }

        // No model loaded, check if any models are available to load
        let models = engine
            .discover_models()
            .await
            .map_err(|e| format!("Failed to discover Parakeet models: {}", e))?;

        let available_models: Vec<_> = models
            .iter()
            .filter(|model| matches!(model.status, crate::parakeet_engine::ModelStatus::Available))
            .collect();

        if available_models.is_empty() {
            return Err(
                "No Parakeet models are available. Please download a model to enable fast transcription."
                    .to_string(),
            );
        }

        // Try to load the first available model (prefer int8 for speed)
        let first_model = available_models.iter()
            .find(|m| m.quantization == crate::parakeet_engine::QuantizationType::Int8)
            .or_else(|| available_models.first())
            .unwrap();

        engine
            .load_model(&first_model.name)
            .await
            .map_err(|e| format!("Failed to load Parakeet model {}: {}", first_model.name, e))?;

        Ok(first_model.name.clone())
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

/// Internal version of parakeet_validate_model_ready that respects user's transcript config
/// This matches whisper_validate_model_ready_with_config for consistency
pub async fn parakeet_validate_model_ready_with_config<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<String, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // Check if a model is currently loaded
        if engine.is_model_loaded().await {
            if let Some(current_model) = engine.get_current_model().await {
                log::info!("Parakeet model already loaded: {}", current_model);
                return Ok(current_model);
            }
        }

        // No model loaded - try to load user's configured model from transcript config
        let model_to_load = match crate::api::api::api_get_transcript_config(
            app.clone(),
            app.state(),
            None,
        )
        .await
        {
            Ok(Some(config)) => {
                log::info!(
                    "Got transcript config from API - provider: {}, model: {}",
                    config.provider,
                    config.model
                );
                if config.provider == "parakeet" && !config.model.is_empty() {
                    log::info!("Using user's configured Parakeet model: {}", config.model);
                    Some(config.model)
                } else {
                    log::info!(
                        "API config uses non-Parakeet provider ({}) or empty model, will auto-select",
                        config.provider
                    );
                    None
                }
            }
            Ok(None) => {
                log::info!("No transcript config found in API, will auto-select Parakeet model");
                None
            }
            Err(e) => {
                log::warn!(
                    "Failed to get transcript config from API: {}, will auto-select Parakeet model",
                    e
                );
                None
            }
        };

        // Check available models
        let models = engine
            .discover_models()
            .await
            .map_err(|e| format!("Failed to discover Parakeet models: {}", e))?;

        let available_models: Vec<_> = models
            .iter()
            .filter(|model| matches!(model.status, crate::parakeet_engine::ModelStatus::Available))
            .collect();

        if available_models.is_empty() {
            return Err(
                "No Parakeet models are available. Please download a model to enable fast transcription."
                    .to_string(),
            );
        }

        // Try to load user's configured model if specified
        let model_name = if let Some(configured_model) = model_to_load {
            // Check if configured model is available
            if available_models.iter().any(|m| m.name == configured_model) {
                log::info!("Loading user's configured Parakeet model: {}", configured_model);
                configured_model
            } else {
                log::warn!(
                    "Configured Parakeet model '{}' not found, falling back to first available int8 model",
                    configured_model
                );
                // Prefer int8 quantization for best speed/quality tradeoff
                available_models
                    .iter()
                    .find(|m| m.quantization == crate::parakeet_engine::QuantizationType::Int8)
                    .or_else(|| available_models.first())
                    .unwrap()
                    .name
                    .clone()
            }
        } else {
            // No configured model, prefer int8 for best speed/quality balance
            log::info!("No configured model, loading first available int8 Parakeet model");
            available_models
                .iter()
                .find(|m| m.quantization == crate::parakeet_engine::QuantizationType::Int8)
                .or_else(|| available_models.first())
                .unwrap()
                .name
                .clone()
        };

        engine
            .load_model(&model_name)
            .await
            .map_err(|e| format!("Failed to load Parakeet model {}: {}", model_name, e))?;

        Ok(model_name)
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_transcribe_audio(audio_data: Vec<f32>) -> Result<String, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        engine
            .transcribe_audio(audio_data)
            .await
            .map_err(|e| format!("Parakeet transcription failed: {}", e))
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_get_models_directory() -> Result<String, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        let path = engine.get_models_directory().await;
        Ok(path.to_string_lossy().to_string())
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_download_model<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // Create progress callback that emits detailed events
        let app_handle_clone = app_handle.clone();
        let model_name_clone = model_name.clone();

        let progress_callback = Box::new(move |progress: DownloadProgress| {
            log::info!(
                "Parakeet download progress for {}: {:.1} MB / {:.1} MB ({:.1} MB/s) - {}%",
                model_name_clone, progress.downloaded_mb, progress.total_mb,
                progress.speed_mbps, progress.percent
            );

            // Emit download progress event with detailed info
            if let Err(e) = app_handle_clone.emit(
                "parakeet-model-download-progress",
                serde_json::json!({
                    "modelName": model_name_clone,
                    "progress": progress.percent,
                    "downloaded_bytes": progress.downloaded_bytes,
                    "total_bytes": progress.total_bytes,
                    "downloaded_mb": progress.downloaded_mb,
                    "total_mb": progress.total_mb,
                    "speed_mbps": progress.speed_mbps,
                    "status": if progress.percent == 100 { "completed" } else { "downloading" }
                }),
            ) {
                log::error!("Failed to emit parakeet download progress event: {}", e);
            }
        });

        // Ensure models are discovered before downloading
        // This populates available_models so we don't get "Model not found" error
        if let Err(e) = engine.discover_models().await {
            log::warn!("Failed to discover models before download: {}", e);
            // Continue anyway, maybe it will work if the model is already known
        }

        let result = engine
            .download_model_detailed(&model_name, Some(progress_callback))
            .await;

        match result {
            Ok(()) => {
                // Emit completion event
                if let Err(e) = app_handle.emit(
                    "parakeet-model-download-complete",
                    serde_json::json!({
                        "modelName": model_name
                    }),
                ) {
                    log::error!("Failed to emit parakeet download complete event: {}", e);
                }

                // Update tray menu to reflect model is now available
                log::info!("Parakeet model download complete - updating tray menu");
                crate::tray::update_tray_menu(&app_handle);

                Ok(())
            }
            Err(e) => {
                // Emit error event
                if let Err(emit_e) = app_handle.emit(
                    "parakeet-model-download-error",
                    serde_json::json!({
                        "modelName": model_name,
                        "error": e.to_string()
                    }),
                ) {
                    log::error!("Failed to emit parakeet download error event: {}", emit_e);
                }
                Err(format!("Failed to download Parakeet model: {}", e))
            }
        }
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_cancel_download<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        engine
            .cancel_download(&model_name)
            .await
            .map_err(|e| format!("Failed to cancel Parakeet download: {}", e))?;

        // Emit cancellation event to update UI (global toast and component state)
        let _ = app_handle.emit(
            "parakeet-model-download-progress",
            serde_json::json!({
                "modelName": model_name,
                "progress": 0,
                "status": "cancelled"
            }),
        );

        log::info!("Parakeet download cancelled: {}", model_name);
        Ok(())
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_retry_download<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    log::info!("Retrying download for: {}", model_name);

    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // DEFENSIVE: Ensure clean state before retry
        // This handles any edge cases where error handler didn't complete
        {
            let mut active = engine.active_downloads.write().await;
            if active.contains(&model_name) {
                log::warn!("Retry: Model {} was still in active downloads, removing", model_name);
                active.remove(&model_name);
            }
        }

        // DEFENSIVE: Force model status to Missing to allow fresh download
        {
            let mut models = engine.available_models.write().await;
            if let Some(model) = models.get_mut(&model_name) {
                log::info!("Retry: Resetting model {} status from {:?} to Missing", model_name, model.status);
                model.status = ModelStatus::Missing;
            }
        }

        // Rediscover models to refresh state based on disk files
        let _ = engine.discover_models().await;

        // Call regular download (emits events)
        parakeet_download_model(app_handle, model_name).await
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

#[command]
pub async fn parakeet_delete_corrupted_model(model_name: String) -> Result<String, String> {
    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        engine
            .delete_model(&model_name)
            .await
            .map_err(|e| format!("Failed to delete Parakeet model: {}", e))
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}

/// Open the Parakeet models folder in the system file explorer
#[command]
pub async fn open_parakeet_models_folder() -> Result<(), String> {
    let models_dir = get_models_directory()
        .ok_or_else(|| "Parakeet models directory not initialized".to_string())?
        .join("parakeet");

    // Ensure directory exists before trying to open it
    if !models_dir.exists() {
        std::fs::create_dir_all(&models_dir)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let folder_path = models_dir.to_string_lossy().to_string();

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

    log::info!("Opened Parakeet models folder: {}", folder_path);
    Ok(())
}

// ============================================================================
// CUSTOM HUGGINGFACE MODEL COMMANDS
// ============================================================================

/// Inspect a HuggingFace model repository to get metadata, format, and size.
#[command]
pub async fn parakeet_inspect_huggingface_model(
    repo_id: String,
) -> Result<serde_json::Value, String> {
    log::info!("Inspecting HuggingFace model: {}", repo_id);

    let result = huggingface_api::inspect_model(&repo_id)
        .await
        .map_err(|e| format!("Failed to inspect model '{}': {}", repo_id, e))?;

    serde_json::to_value(&result)
        .map_err(|e| format!("Failed to serialize inspection result: {}", e))
}

/// Add a custom model from HuggingFace by repo ID.
/// This registers the model in the catalog and starts the download.
#[command]
pub async fn parakeet_add_custom_model<R: Runtime>(
    app_handle: AppHandle<R>,
    repo_id: String,
    label: Option<String>,
) -> Result<serde_json::Value, String> {
    log::info!("Adding custom HuggingFace model: {}", repo_id);

    // Step 1: Inspect the model to get metadata
    let inspection = huggingface_api::inspect_model(&repo_id)
        .await
        .map_err(|e| format!("Failed to inspect model '{}': {}", repo_id, e))?;

    // Step 2: Validate it's a compatible model
    if inspection.format == ModelFormat::Unknown {
        return Err(format!(
            "Model '{}' has no recognized model files (ONNX, Safetensors, MLX, or NeMo). \
             Found files: {:?}",
            repo_id,
            inspection.model_files
        ));
    }

    // Step 3: Create and register the custom model entry
    let entry = CustomModelCatalogEntry::from_huggingface(
        &repo_id,
        inspection.format,
        inspection.model_files.clone(),
        inspection.total_size_mb,
        label,
        None,
    );

    let model_id = entry.model_id.clone();
    register_custom_model(entry.clone())
        .map_err(|e| format!("Failed to register custom model: {}", e))?;

    // Step 4: Start download in background
    let models_dir = get_models_directory()
        .ok_or_else(|| "Models directory not initialized".to_string())?
        .join("parakeet")
        .join("custom")
        .join(repo_id.replace('/', "_"));

    let app_clone = app_handle.clone();
    let model_id_clone = model_id.clone();
    let files_to_download = inspection.model_files.clone();
    let repo_id_clone = repo_id.clone();

    tokio::spawn(async move {
        // Update status to downloading
        let _ = update_custom_model_status(&model_id_clone, CustomModelStatus::Downloading { progress: 0 });

        let progress_callback: Box<dyn Fn(huggingface_api::HfDownloadProgress) + Send + Sync> = {
            let app = app_clone.clone();
            let mid = model_id_clone.clone();
            Box::new(move |progress: huggingface_api::HfDownloadProgress| {
                // Update registry status
                let _ = update_custom_model_status(
                    &mid,
                    CustomModelStatus::Downloading { progress: progress.overall_percent },
                );

                // Emit progress event to frontend
                let _ = app.emit(
                    "custom-model-download-progress",
                    serde_json::json!({
                        "modelId": mid,
                        "currentFile": progress.current_file,
                        "fileIndex": progress.file_index,
                        "totalFiles": progress.total_files,
                        "downloadedBytes": progress.downloaded_bytes,
                        "totalBytes": progress.total_bytes,
                        "overallPercent": progress.overall_percent,
                        "speedMbps": progress.speed_mbps,
                    }),
                );
            })
        };

        match huggingface_api::download_model_files(
            &repo_id_clone,
            &files_to_download,
            &models_dir,
            Some(progress_callback),
        ).await {
            Ok(_) => {
                log::info!("Custom model '{}' downloaded successfully", model_id_clone);
                let _ = update_custom_model_status(&model_id_clone, CustomModelStatus::Ready);

                let _ = app_clone.emit(
                    "custom-model-download-complete",
                    serde_json::json!({
                        "modelId": model_id_clone,
                    }),
                );
            }
            Err(e) => {
                log::error!("Failed to download custom model '{}': {}", model_id_clone, e);
                let _ = update_custom_model_status(
                    &model_id_clone,
                    CustomModelStatus::Error(e.to_string()),
                );

                let _ = app_clone.emit(
                    "custom-model-download-error",
                    serde_json::json!({
                        "modelId": model_id_clone,
                        "error": e.to_string(),
                    }),
                );
            }
        }
    });

    // Return the registered model info
    serde_json::to_value(&serde_json::json!({
        "modelId": model_id,
        "format": format!("{}", inspection.format),
        "sizeMb": inspection.total_size_mb,
        "files": inspection.model_files,
        "isSttModel": inspection.is_stt_model,
        "status": "downloading",
    }))
    .map_err(|e| format!("Failed to serialize response: {}", e))
}

/// Add a custom model from a local directory path.
#[command]
pub async fn parakeet_add_local_model(
    path: String,
    label: Option<String>,
) -> Result<serde_json::Value, String> {
    log::info!("Adding local model from path: {}", path);

    let model_path = PathBuf::from(&path);

    // Validate the local path
    let files = huggingface_api::validate_local_model_path(&model_path)
        .await
        .map_err(|e| format!("Invalid model path: {}", e))?;

    let format = detect_model_format(&files);
    if format == ModelFormat::Unknown {
        return Err(format!(
            "No recognized model files found at '{}'. Expected ONNX, Safetensors, MLX, or NeMo files.",
            path
        ));
    }

    // Calculate total size
    let total_size: u64 = files.iter().filter_map(|f| {
        std::fs::metadata(model_path.join(f)).ok().map(|m| m.len())
    }).sum();
    let size_mb = (total_size / (1024 * 1024)) as u32;

    let entry = CustomModelCatalogEntry::from_local_path(
        model_path,
        format,
        files.clone(),
        size_mb,
        label,
    );

    // Local models are immediately ready (no download needed)
    let model_id = entry.model_id.clone();
    let mut entry = entry;
    entry.status = CustomModelStatus::Ready;

    register_custom_model(entry)
        .map_err(|e| format!("Failed to register local model: {}", e))?;

    Ok(serde_json::json!({
        "modelId": model_id,
        "format": format!("{}", format),
        "sizeMb": size_mb,
        "files": files,
        "status": "ready",
    }))
}

/// Remove a custom model from the registry.
#[command]
pub async fn parakeet_remove_custom_model(
    model_id: String,
) -> Result<String, String> {
    log::info!("Removing custom model: {}", model_id);

    let entry = unregister_custom_model(&model_id)
        .map_err(|e| format!("Failed to remove custom model: {}", e))?;

    // If it was a HuggingFace model, clean up downloaded files
    if entry.repo.is_some() {
        let models_dir = get_models_directory()
            .ok_or_else(|| "Models directory not initialized".to_string())?
            .join("parakeet")
            .join("custom")
            .join(entry.repo.as_ref().unwrap().replace('/', "_"));

        if models_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&models_dir).await {
                log::warn!("Failed to clean up model files at {}: {}", models_dir.display(), e);
            } else {
                log::info!("Cleaned up model files at {}", models_dir.display());
            }
        }
    }

    Ok(format!("Custom model '{}' removed successfully", model_id))
}

/// Get all registered custom models.
#[command]
pub async fn parakeet_get_custom_models() -> Result<serde_json::Value, String> {
    let models = get_custom_models();
    serde_json::to_value(&models)
        .map_err(|e| format!("Failed to serialize custom models: {}", e))
}

/// Load a custom model for transcription.
#[command]
pub async fn parakeet_load_custom_model(
    model_id: String,
) -> Result<(), String> {
    log::info!("Loading custom model: {}", model_id);

    let custom = lookup_custom_model(&model_id)
        .ok_or_else(|| format!("Custom model '{}' not found", model_id))?;

    if custom.status != CustomModelStatus::Ready {
        return Err(format!(
            "Custom model '{}' is not ready (status: {:?})",
            model_id, custom.status
        ));
    }

    // Only ONNX models can be loaded directly with the Parakeet engine
    if custom.format != ModelFormat::Onnx {
        return Err(format!(
            "Custom model '{}' has format '{}' which requires conversion to ONNX before loading. \
             Please convert the model first.",
            model_id, custom.format
        ));
    }

    // Determine the model directory
    let model_dir = if let Some(local_path) = &custom.local_path {
        local_path.clone()
    } else if let Some(repo) = &custom.repo {
        get_models_directory()
            .ok_or_else(|| "Models directory not initialized".to_string())?
            .join("parakeet")
            .join("custom")
            .join(repo.replace('/', "_"))
    } else {
        return Err("Custom model has no path or repo configured".to_string());
    };

    if !model_dir.exists() {
        return Err(format!("Model directory not found: {}", model_dir.display()));
    }

    let engine = {
        let guard = PARAKEET_ENGINE.lock().unwrap();
        guard.as_ref().cloned()
    };

    if let Some(engine) = engine {
        // Check if the custom model has int8 quantization files
        let has_int8 = model_dir.join("encoder-model.int8.onnx").exists();

        // Use the public load_custom_model method
        engine.load_custom_model(&model_id, &model_dir, has_int8)
            .await
            .map_err(|e| format!("Failed to load custom model: {}", e))?;

        log::info!("Custom model '{}' loaded successfully", model_id);
        Ok(())
    } else {
        Err("Parakeet engine not initialized".to_string())
    }
}
