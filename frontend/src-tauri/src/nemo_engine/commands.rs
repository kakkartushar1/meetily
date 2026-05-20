use crate::nemo_engine::NemoEngine;
use crate::parakeet_engine::{DownloadProgress, ModelInfo, ModelStatus};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{command, AppHandle, Emitter, Manager, Runtime};

pub static NEMO_ENGINE: Mutex<Option<Arc<NemoEngine>>> = Mutex::new(None);

static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static SIDECAR_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .expect("Failed to get app data dir");
    let models_dir = app_data_dir.join("models");

    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            log::error!("Failed to create models directory for NeMo: {}", e);
            return;
        }
    }

    let sidecar_dir = resolve_sidecar_dir(app);

    *MODELS_DIR.lock().unwrap() = Some(models_dir);
    *SIDECAR_DIR.lock().unwrap() = sidecar_dir;
}

fn resolve_sidecar_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("sidecars").join("nemo_asr"));
    }

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("sidecars").join("nemo_asr"));
        candidates.push(current_dir.join("src-tauri").join("sidecars").join("nemo_asr"));
        candidates.push(
            current_dir
                .join("frontend")
                .join("src-tauri")
                .join("sidecars")
                .join("nemo_asr"),
        );
    }

    candidates
        .into_iter()
        .find(|dir| dir.join("server.py").exists())
}

fn get_models_directory() -> Option<PathBuf> {
    MODELS_DIR.lock().unwrap().clone()
}

fn get_sidecar_directory() -> Option<PathBuf> {
    SIDECAR_DIR.lock().unwrap().clone()
}

fn get_engine() -> Result<Arc<NemoEngine>, String> {
    let guard = NEMO_ENGINE.lock().unwrap();
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "NeMo engine not initialized".to_string())
}

#[command]
pub async fn nemo_init() -> Result<(), String> {
    let mut guard = NEMO_ENGINE.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    let engine = NemoEngine::new(get_models_directory(), get_sidecar_directory())
        .map_err(|e| format!("Failed to initialize NeMo engine: {}", e))?;
    *guard = Some(Arc::new(engine));
    Ok(())
}

#[command]
pub async fn nemo_get_available_models() -> Result<Vec<ModelInfo>, String> {
    nemo_init().await?;
    get_engine()?
        .discover_models()
        .await
        .map_err(|e| format!("Failed to discover NeMo models: {}", e))
}

#[command]
pub async fn nemo_has_available_models() -> Result<bool, String> {
    let models = nemo_get_available_models().await?;
    Ok(models
        .iter()
        .any(|model| matches!(model.status, ModelStatus::Available)))
}

#[command]
pub async fn nemo_load_model(model_name: String) -> Result<(), String> {
    nemo_init().await?;
    get_engine()?
        .load_model(&model_name)
        .await
        .map_err(|e| format!("Failed to load NeMo model: {}", e))
}

#[command]
pub async fn nemo_get_current_model() -> Result<Option<String>, String> {
    nemo_init().await?;
    Ok(get_engine()?.get_current_model().await)
}

#[command]
pub async fn nemo_is_model_loaded() -> Result<bool, String> {
    nemo_init().await?;
    Ok(get_engine()?.is_model_loaded().await)
}

#[command]
pub async fn nemo_transcribe_audio(audio_data: Vec<f32>) -> Result<String, String> {
    nemo_init().await?;
    get_engine()?
        .transcribe_audio(audio_data)
        .await
        .map_err(|e| format!("NeMo transcription failed: {}", e))
}

#[command]
pub async fn nemo_get_models_directory() -> Result<String, String> {
    nemo_init().await?;
    Ok(get_engine()?
        .get_models_directory()
        .await
        .to_string_lossy()
        .to_string())
}

#[command]
pub async fn nemo_validate_model_ready<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: Option<String>,
) -> Result<String, String> {
    nemo_validate_model_ready_internal(&app_handle, model_name).await
}

pub async fn nemo_validate_model_ready_internal<R: Runtime>(
    app: &AppHandle<R>,
    model_name: Option<String>,
) -> Result<String, String> {
    nemo_init().await?;

    let model_to_load = match model_name {
        Some(model) if !model.is_empty() => model,
        _ => match crate::api::api::api_get_transcript_config(app.clone(), app.state(), None).await {
            Ok(Some(config))
                if config.provider == "parakeet"
                    && crate::transcription_catalog::is_nemo_model(&config.model) =>
            {
                config.model
            }
            _ => crate::transcription_catalog::NEMO_PARAKEET_RNNT_1_1B.to_string(),
        },
    };

    let models = get_engine()?
        .discover_models()
        .await
        .map_err(|e| format!("Failed to discover NeMo models: {}", e))?;
    let model = models
        .iter()
        .find(|model| model.name == model_to_load)
        .ok_or_else(|| format!("NeMo model '{}' is not supported", model_to_load))?;

    match &model.status {
        ModelStatus::Available => {
            get_engine()?
                .load_model(&model_to_load)
                .await
                .map_err(|e| format!("Failed to load NeMo model {}: {}", model_to_load, e))?;
            Ok(model_to_load)
        }
        ModelStatus::Missing => Err(format!(
            "NeMo model '{}' is not downloaded. Please download it from transcription settings.",
            model_to_load
        )),
        ModelStatus::Downloading { progress } => Err(format!(
            "NeMo model '{}' is currently downloading ({}%). Please wait for it to complete.",
            model_to_load, progress
        )),
        ModelStatus::Error(err) => Err(format!("NeMo model '{}' has an error: {}", model_to_load, err)),
        ModelStatus::Corrupted { .. } => Err(format!(
            "NeMo model '{}' is incomplete or corrupted. Please delete it and download again.",
            model_to_load
        )),
    }
}

#[command]
pub async fn nemo_download_model<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    nemo_download_model_with_event_prefix(app_handle, model_name, "nemo-model").await
}

pub async fn nemo_download_model_with_event_prefix<R: Runtime>(
    app_handle: AppHandle<R>,
    model_name: String,
    event_prefix: &'static str,
) -> Result<(), String> {
    nemo_init().await?;

    let app_for_progress = app_handle.clone();
    let progress_model_name = model_name.clone();
    let progress_event = format!("{}-download-progress", event_prefix);

    let progress_callback = Box::new(move |progress: DownloadProgress| {
        let _ = app_for_progress.emit(
            &progress_event,
            serde_json::json!({
                "modelName": progress_model_name.clone(),
                "progress": progress.percent,
                "downloaded_bytes": progress.downloaded_bytes,
                "total_bytes": progress.total_bytes,
                "downloaded_mb": progress.downloaded_mb,
                "total_mb": progress.total_mb,
                "speed_mbps": progress.speed_mbps,
                "status": if progress.percent == 100 { "completed" } else { "downloading" }
            }),
        );
    });

    let result = get_engine()?
        .download_model_detailed(&model_name, Some(progress_callback))
        .await;

    match result {
        Ok(()) => {
            let _ = app_handle.emit(
                &format!("{}-download-complete", event_prefix),
                serde_json::json!({ "modelName": model_name }),
            );
            Ok(())
        }
        Err(e) => {
            let _ = app_handle.emit(
                &format!("{}-download-error", event_prefix),
                serde_json::json!({
                    "modelName": model_name,
                    "error": e.to_string()
                }),
            );
            Err(format!("Failed to download NeMo model: {}", e))
        }
    }
}

#[command]
pub async fn nemo_cancel_download(model_name: String) -> Result<(), String> {
    nemo_init().await?;
    get_engine()?
        .cancel_download(&model_name)
        .await
        .map_err(|e| format!("Failed to cancel NeMo download: {}", e))
}

#[command]
pub async fn nemo_delete_model(model_name: String) -> Result<String, String> {
    nemo_init().await?;
    get_engine()?
        .delete_model(&model_name)
        .await
        .map_err(|e| format!("Failed to delete NeMo model: {}", e))
}

#[command]
pub async fn open_nemo_models_folder() -> Result<(), String> {
    let models_dir = get_models_directory()
        .ok_or_else(|| "NeMo models directory not initialized".to_string())?
        .join("nemo");

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

    Ok(())
}
