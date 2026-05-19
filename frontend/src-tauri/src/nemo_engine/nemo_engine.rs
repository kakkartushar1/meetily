//! NeMo ASR Engine - manages the Python NeMo sidecar process.
//!
//! This module handles:
//! - Spawning/stopping the Python FastAPI sidecar
//! - HTTP communication with the sidecar
//! - Model download, load, transcribe, unload lifecycle
//! - Health checking and auto-restart

use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

// ============================================================================
// CONSTANTS
// ============================================================================

const SIDECAR_PORT: u16 = 9876;
const SIDECAR_HOST: &str = "127.0.0.1";
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600); // 10 min for large transcriptions

// ============================================================================
// TYPES
// ============================================================================

/// Status of a NeMo model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NemoModelStatus {
    Available,
    Missing,
    Downloading { progress: u8 },
    Error(String),
}

/// Information about a NeMo model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NemoModelInfo {
    pub model_id: String,
    pub filename: String,
    pub size_mb: u32,
    pub label: String,
    pub description: String,
    pub status: NemoModelStatus,
}

/// Response from the sidecar /health endpoint.
#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
    device: String,
    model_loaded: Option<String>,
}

/// Response from the sidecar /models endpoint.
#[derive(Debug, Deserialize)]
struct SidecarModelInfo {
    model_id: String,
    filename: String,
    size_bytes: u64,
    ready: bool,
}

/// Response from the sidecar /download endpoint.
#[derive(Debug, Deserialize)]
struct DownloadResponse {
    status: String,
    path: Option<String>,
    size_bytes: Option<u64>,
}

/// Response from the sidecar /load endpoint.
#[derive(Debug, Deserialize)]
struct LoadResponse {
    status: String,
    model_id: Option<String>,
    device: Option<String>,
}

/// Response from the sidecar /transcribe endpoint.
#[derive(Debug, Deserialize)]
struct TranscribeResponse {
    text: String,
}

/// Response from the sidecar /unload endpoint.
#[derive(Debug, Deserialize)]
struct UnloadResponse {
    status: String,
    model_id: Option<String>,
}

// ============================================================================
// NEMO ENGINE
// ============================================================================

pub struct NemoEngine {
    /// Child process handle for the Python sidecar
    child_process: Arc<RwLock<Option<Child>>>,
    /// Whether the sidecar is healthy
    is_healthy: Arc<AtomicBool>,
    /// HTTP client for sidecar communication
    http_client: reqwest::Client,
    /// Models directory root
    models_dir: PathBuf,
    /// Currently loaded model ID
    current_model_id: Arc<RwLock<Option<String>>>,
    /// Cancel download flag
    cancel_download_flag: Arc<RwLock<Option<String>>>,
}

impl NemoEngine {
    /// Create a new NeMo engine.
    pub fn new(models_dir: Option<PathBuf>) -> Result<Self> {
        let models_dir = if let Some(dir) = models_dir {
            dir.join("nemo")
        } else {
            let base = if cfg!(debug_assertions) {
                std::env::current_dir()?.join("models")
            } else {
                dirs::data_dir()
                    .or_else(dirs::home_dir)
                    .ok_or_else(|| anyhow!("Could not find system data directory"))?
                    .join("Meetily")
                    .join("models")
            };
            base.join("nemo")
        };

        info!("NemoEngine using models directory: {}", models_dir.display());

        if !models_dir.exists() {
            std::fs::create_dir_all(&models_dir)?;
        }

        let http_client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            child_process: Arc::new(RwLock::new(None)),
            is_healthy: Arc::new(AtomicBool::new(false)),
            http_client,
            models_dir,
            current_model_id: Arc::new(RwLock::new(None)),
            cancel_download_flag: Arc::new(RwLock::new(None)),
        })
    }

    /// Base URL for the sidecar HTTP service.
    fn base_url(&self) -> String {
        format!("http://{}:{}", SIDECAR_HOST, SIDECAR_PORT)
    }

    // ========================================================================
    // SIDECAR LIFECYCLE
    // ========================================================================

    /// Ensure the Python sidecar is running.
    pub async fn ensure_sidecar_running(&self) -> Result<()> {
        if self.is_healthy.load(Ordering::SeqCst) {
            // Quick health check
            if self.health_check().await.is_ok() {
                return Ok(());
            }
        }

        self.start_sidecar().await
    }

    /// Start the Python NeMo sidecar process.
    async fn start_sidecar(&self) -> Result<()> {
        // Stop existing process if any
        self.stop_sidecar().await?;

        info!("Starting NeMo ASR sidecar...");

        let sidecar_script = self.find_sidecar_script()?;
        info!("Sidecar script: {}", sidecar_script.display());

        let python = self.find_python()?;
        info!("Python executable: {}", python.display());

        let mut cmd = Command::new(&python);
        cmd.arg(&sidecar_script)
            .env("NEMO_MODELS_DIR", &self.models_dir)
            .env("NEMO_ASR_PORT", SIDECAR_PORT.to_string())
            .env("NEMO_ASR_HOST", SIDECAR_HOST)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| {
            anyhow!(
                "Failed to start NeMo sidecar. Ensure Python is installed with NeMo dependencies. Error: {}",
                e
            )
        })?;

        {
            let mut guard = self.child_process.write().await;
            *guard = Some(child);
        }

        // Wait for sidecar to become healthy
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > STARTUP_TIMEOUT {
                self.stop_sidecar().await?;
                return Err(anyhow!(
                    "NeMo sidecar failed to start within {:?}",
                    STARTUP_TIMEOUT
                ));
            }

            match self.health_check().await {
                Ok(_) => {
                    self.is_healthy.store(true, Ordering::SeqCst);
                    info!("NeMo sidecar is healthy");
                    return Ok(());
                }
                Err(_) => {
                    // Check if process has exited
                    let mut guard = self.child_process.write().await;
                    if let Some(ref mut child) = *guard {
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                *guard = None;
                                return Err(anyhow!(
                                    "NeMo sidecar exited during startup with status: {}",
                                    status
                                ));
                            }
                            Ok(None) => {} // Still running, keep waiting
                            Err(e) => {
                                warn!("Failed to check sidecar status: {}", e);
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Stop the Python sidecar process.
    pub async fn stop_sidecar(&self) -> Result<()> {
        let mut guard = self.child_process.write().await;
        if let Some(mut child) = guard.take() {
            info!("Stopping NeMo sidecar...");

            // Try graceful shutdown first
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(status)) => {
                    info!("NeMo sidecar exited with status: {}", status);
                }
                _ => {
                    warn!("NeMo sidecar didn't exit gracefully, killing");
                    let _ = child.kill().await;
                }
            }
        }

        self.is_healthy.store(false, Ordering::SeqCst);
        *self.current_model_id.write().await = None;

        Ok(())
    }

    /// Health check against the sidecar.
    async fn health_check(&self) -> Result<HealthResponse> {
        let url = format!("{}/health", self.base_url());
        let resp = self
            .http_client
            .get(&url)
            .timeout(HEALTH_CHECK_TIMEOUT)
            .send()
            .await
            .map_err(|e| anyhow!("Health check failed: {}", e))?;

        let health: HealthResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse health response: {}", e))?;

        Ok(health)
    }

    /// Find the Python executable.
    fn find_python(&self) -> Result<PathBuf> {
        // Check for venv in sidecar directory
        let sidecar_dir = self.find_sidecar_dir()?;
        let venv_python = if cfg!(windows) {
            sidecar_dir.join(".venv").join("Scripts").join("python.exe")
        } else {
            sidecar_dir.join(".venv").join("bin").join("python")
        };

        if venv_python.exists() {
            return Ok(venv_python);
        }

        // Check environment variable
        if let Ok(python) = std::env::var("NEMO_PYTHON") {
            let path = PathBuf::from(python);
            if path.exists() {
                return Ok(path);
            }
        }

        // Fallback to system Python
        let candidates = if cfg!(windows) {
            vec!["python.exe", "python3.exe"]
        } else {
            vec!["python3", "python"]
        };

        for candidate in candidates {
            if which::which(candidate).is_ok() {
                return Ok(PathBuf::from(candidate));
            }
        }

        Err(anyhow!(
            "Python not found. Install Python 3.10+ and NeMo dependencies."
        ))
    }

    /// Find the sidecar directory.
    fn find_sidecar_dir(&self) -> Result<PathBuf> {
        // Check environment variable
        if let Ok(dir) = std::env::var("NEMO_SIDECAR_DIR") {
            let path = PathBuf::from(dir);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check relative to executable
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let bundled = exe_dir.join("sidecars").join("nemo_asr");
                if bundled.exists() {
                    return Ok(bundled);
                }
            }
        }

        // Dev mode: relative to CARGO_MANIFEST_DIR
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let dev_path = PathBuf::from(&manifest_dir)
                .join("sidecars")
                .join("nemo_asr");
            if dev_path.exists() {
                return Ok(dev_path);
            }
        }

        Err(anyhow!("NeMo sidecar directory not found"))
    }

    /// Find the sidecar server.py script.
    fn find_sidecar_script(&self) -> Result<PathBuf> {
        let dir = self.find_sidecar_dir()?;
        let script = dir.join("server.py");
        if script.exists() {
            Ok(script)
        } else {
            Err(anyhow!("NeMo sidecar server.py not found in {:?}", dir))
        }
    }

    // ========================================================================
    // MODEL OPERATIONS
    // ========================================================================

    /// Get available NeMo models (from catalog + local state).
    pub async fn get_available_models(&self) -> Result<Vec<NemoModelInfo>> {
        let mut models = Vec::new();

        // Get catalog entries for NeMo runtime
        for entry in crate::model_catalog::models_for_runtime(crate::model_catalog::ModelRuntime::Nemo) {
            let model_dir = self.model_dir_for(entry.model_id);
            let nemo_file = self.find_nemo_file(&model_dir);

            let status = if let Some(ref path) = nemo_file {
                if path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                    NemoModelStatus::Available
                } else {
                    NemoModelStatus::Missing
                }
            } else {
                NemoModelStatus::Missing
            };

            models.push(NemoModelInfo {
                model_id: entry.model_id.to_string(),
                filename: entry.file.unwrap_or("").to_string(),
                size_mb: entry.size_mb,
                label: entry.label.to_string(),
                description: entry.description.to_string(),
                status,
            });
        }

        Ok(models)
    }

    /// Download a NeMo model via the sidecar.
    pub async fn download_model(&self, model_id: &str) -> Result<()> {
        let entry = crate::model_catalog::lookup_model(model_id)
            .ok_or_else(|| anyhow!("Model '{}' not found in catalog", model_id))?;

        let filename = entry
            .file
            .ok_or_else(|| anyhow!("Model '{}' has no filename in catalog", model_id))?;

        let repo_id = entry
            .repo
            .ok_or_else(|| anyhow!("Model '{}' has no repo in catalog", model_id))?;

        // Ensure sidecar is running
        self.ensure_sidecar_running().await?;

        // Clear cancel flag
        *self.cancel_download_flag.write().await = None;

        let url = format!("{}/download", self.base_url());
        let resp = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({
                "repo_id": repo_id,
                "filename": filename,
            }))
            .send()
            .await
            .map_err(|e| anyhow!("Download request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Download failed (HTTP {}): {}", status, body));
        }

        let download_resp: DownloadResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse download response: {}", e))?;

        info!(
            "Download result for {}: status={}",
            model_id, download_resp.status
        );

        Ok(())
    }

    /// Cancel an ongoing download.
    pub async fn cancel_download(&self, model_id: &str) -> Result<()> {
        *self.cancel_download_flag.write().await = Some(model_id.to_string());
        info!("Download cancellation requested for: {}", model_id);
        Ok(())
    }

    /// Load a NeMo model into memory via the sidecar.
    pub async fn load_model(&self, model_id: &str) -> Result<()> {
        // Ensure sidecar is running
        self.ensure_sidecar_running().await?;

        let url = format!("{}/load", self.base_url());
        let resp = self
            .http_client
            .post(&url)
            .json(&serde_json::json!({ "model_id": model_id }))
            .send()
            .await
            .map_err(|e| anyhow!("Load request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Load failed (HTTP {}): {}", status, body));
        }

        let load_resp: LoadResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse load response: {}", e))?;

        *self.current_model_id.write().await = Some(model_id.to_string());
        info!(
            "NeMo model loaded: {} (device: {})",
            model_id,
            load_resp.device.as_deref().unwrap_or("unknown")
        );

        Ok(())
    }

    /// Transcribe audio using the loaded NeMo model.
    ///
    /// `audio_data` should be 16 kHz mono f32 samples.
    pub async fn transcribe_audio(&self, audio_data: Vec<f32>) -> Result<String> {
        if !self.is_healthy.load(Ordering::SeqCst) {
            return Err(anyhow!("NeMo sidecar is not running"));
        }

        // Write audio to a temp WAV file
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("nemo_audio_{}.wav", uuid::Uuid::new_v4()));

        // Convert f32 samples to i16 WAV
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(&temp_path, spec)
            .map_err(|e| anyhow!("Failed to create temp WAV: {}", e))?;

        for sample in &audio_data {
            let s16 = (*sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
            writer
                .write_sample(s16)
                .map_err(|e| anyhow!("Failed to write WAV sample: {}", e))?;
        }
        writer
            .finalize()
            .map_err(|e| anyhow!("Failed to finalize WAV: {}", e))?;

        // Send to sidecar
        let url = format!("{}/transcribe", self.base_url());
        let file_bytes = tokio::fs::read(&temp_path).await?;

        let form = reqwest::multipart::Form::new().part(
            "audio",
            reqwest::multipart::Part::bytes(file_bytes)
                .file_name("audio.wav")
                .mime_str("audio/wav")?,
        );

        let resp = self
            .http_client
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| anyhow!("Transcribe request failed: {}", e))?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_path).await;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Transcription failed (HTTP {}): {}", status, body));
        }

        let transcribe_resp: TranscribeResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse transcribe response: {}", e))?;

        Ok(transcribe_resp.text)
    }

    /// Unload the current model from the sidecar.
    pub async fn unload_model(&self) -> Result<()> {
        if !self.is_healthy.load(Ordering::SeqCst) {
            *self.current_model_id.write().await = None;
            return Ok(());
        }

        let url = format!("{}/unload", self.base_url());
        let resp = self
            .http_client
            .post(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Unload request failed: {}", e))?;

        if resp.status().is_success() {
            let unload_resp: UnloadResponse = resp.json().await.unwrap_or(UnloadResponse {
                status: "unknown".to_string(),
                model_id: None,
            });
            info!("NeMo model unloaded: {:?}", unload_resp.model_id);
        }

        *self.current_model_id.write().await = None;
        Ok(())
    }

    /// Check if a model is loaded.
    pub async fn is_model_loaded(&self) -> bool {
        self.current_model_id.read().await.is_some()
    }

    /// Get the currently loaded model ID.
    pub async fn get_current_model(&self) -> Option<String> {
        self.current_model_id.read().await.clone()
    }

    /// Get the models directory.
    pub fn get_models_directory(&self) -> &PathBuf {
        &self.models_dir
    }

    /// Validate that a NeMo model is ready for transcription.
    pub async fn validate_model_ready(&self, model_id: &str) -> Result<()> {
        // Check if model files exist locally
        let model_dir = self.model_dir_for(model_id);
        let nemo_file = self.find_nemo_file(&model_dir);

        match nemo_file {
            Some(path) if path.exists() => {
                info!("NeMo model files found: {}", path.display());
                Ok(())
            }
            _ => Err(anyhow!(
                "NeMo model '{}' is not downloaded. Please download it first.",
                model_id
            )),
        }
    }

    /// Delete a downloaded model.
    pub async fn delete_model(&self, model_id: &str) -> Result<()> {
        let model_dir = self.model_dir_for(model_id);
        if model_dir.exists() {
            tokio::fs::remove_dir_all(&model_dir).await?;
            info!("Deleted NeMo model directory: {}", model_dir.display());
        }

        // Unload if currently loaded
        if self.current_model_id.read().await.as_deref() == Some(model_id) {
            self.unload_model().await?;
        }

        Ok(())
    }

    // ========================================================================
    // HELPERS
    // ========================================================================

    /// Get the local directory for a model.
    fn model_dir_for(&self, model_id: &str) -> PathBuf {
        let safe_name = model_id.replace('/', "--");
        self.models_dir.join(safe_name)
    }

    /// Find a .nemo file in a model directory.
    fn find_nemo_file(&self, model_dir: &PathBuf) -> Option<PathBuf> {
        if !model_dir.exists() {
            return None;
        }
        if let Ok(entries) = std::fs::read_dir(model_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("nemo") {
                    return Some(path);
                }
            }
        }
        None
    }
}

impl Drop for NemoEngine {
    fn drop(&mut self) {
        // Best-effort cleanup; can't do async in Drop
        debug!("NemoEngine dropped");
    }
}
