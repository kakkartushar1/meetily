use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};

use crate::parakeet_engine::{DownloadProgress, ModelInfo, ModelStatus, QuantizationType};
use crate::transcription_catalog::{self, TranscriptionModelCatalogEntry};

const DEFAULT_NEMO_PORT: u16 = 5877;
const SIDECAR_STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

#[derive(Debug, Serialize)]
struct LoadRequest<'a> {
    model_id: &'a str,
    model_path: &'a str,
}

#[derive(Debug, Deserialize)]
struct LoadResponse {
    status: String,
}

#[derive(Debug, Serialize)]
struct TranscribeRequest<'a> {
    audio_path: &'a str,
}

#[derive(Debug, Deserialize)]
struct TranscribeResponse {
    text: String,
}

/// Runtime manager for `.nemo` ASR models.
pub struct NemoEngine {
    models_dir: PathBuf,
    sidecar_dir: Option<PathBuf>,
    current_model_name: Arc<RwLock<Option<String>>>,
    active_downloads: Arc<RwLock<HashSet<String>>>,
    cancel_download_flag: Arc<RwLock<Option<String>>>,
    child_process: Arc<Mutex<Option<Child>>>,
    port: u16,
}

impl NemoEngine {
    pub fn new(models_root: Option<PathBuf>, sidecar_dir: Option<PathBuf>) -> Result<Self> {
        let models_dir = if let Some(root) = models_root {
            root.join("nemo")
        } else {
            dirs::data_dir()
                .or_else(|| dirs::home_dir())
                .ok_or_else(|| anyhow!("Could not find system data directory"))?
                .join("Meetily")
                .join("models")
                .join("nemo")
        };

        if !models_dir.exists() {
            std::fs::create_dir_all(&models_dir)?;
        }

        let port = std::env::var("MEETILY_NEMO_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_NEMO_PORT);

        Ok(Self {
            models_dir,
            sidecar_dir,
            current_model_name: Arc::new(RwLock::new(None)),
            active_downloads: Arc::new(RwLock::new(HashSet::new())),
            cancel_download_flag: Arc::new(RwLock::new(None)),
            child_process: Arc::new(Mutex::new(None)),
            port,
        })
    }

    pub async fn discover_models(&self) -> Result<Vec<ModelInfo>> {
        let active_downloads = self.active_downloads.read().await;
        let mut models = Vec::new();

        for entry in transcription_catalog::nemo_models() {
            let model_path = self.model_file_path(entry)?;
            let status = if active_downloads.contains(entry.model_id) {
                ModelStatus::Downloading { progress: 0 }
            } else {
                self.model_status(entry, &model_path).await
            };

            models.push(ModelInfo {
                name: entry.model_id.to_string(),
                path: model_path,
                size_mb: entry.size_mb,
                quantization: QuantizationType::FP32,
                runtime: "nemo".to_string(),
                speed: entry.speed.to_string(),
                status,
                description: entry.description.to_string(),
            });
        }

        Ok(models)
    }

    pub async fn load_model(&self, model_name: &str) -> Result<()> {
        let entry = self.catalog_entry(model_name)?;
        let model_path = self.model_file_path(entry)?;

        match self.model_status(entry, &model_path).await {
            ModelStatus::Available => {}
            ModelStatus::Missing => return Err(anyhow!("NeMo model {} is not downloaded", model_name)),
            ModelStatus::Downloading { .. } => {
                return Err(anyhow!("NeMo model {} is currently downloading", model_name));
            }
            ModelStatus::Error(err) => return Err(anyhow!("NeMo model {} has error: {}", model_name, err)),
            ModelStatus::Corrupted { file_size, expected_min_size } => {
                return Err(anyhow!(
                    "NeMo model {} is incomplete: {} bytes (expected at least {})",
                    model_name,
                    file_size,
                    expected_min_size
                ));
            }
        }

        if self.get_current_model().await.as_deref() == Some(model_name) {
            return Ok(());
        }

        self.ensure_sidecar_running().await?;

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/load", self.base_url()))
            .json(&LoadRequest {
                model_id: entry.model_id,
                model_path: &model_path.to_string_lossy(),
            })
            .send()
            .await
            .map_err(|e| anyhow!("Failed to call NeMo sidecar /load: {}", e))?;

        if !response.status().is_success() {
            return Err(anyhow!("NeMo sidecar /load failed with status {}", response.status()));
        }

        let body: LoadResponse = response
            .json()
            .await
            .map_err(|e| anyhow!("Invalid NeMo sidecar /load response: {}", e))?;

        if body.status != "loaded" {
            return Err(anyhow!("Unexpected NeMo load status: {}", body.status));
        }

        *self.current_model_name.write().await = Some(model_name.to_string());
        Ok(())
    }

    pub async fn transcribe_audio(&self, audio_data: Vec<f32>) -> Result<String> {
        let current_model = self
            .get_current_model()
            .await
            .ok_or_else(|| anyhow!("No NeMo model loaded. Please load a model first."))?;

        if audio_data.is_empty() {
            return Ok(String::new());
        }

        self.ensure_sidecar_running().await?;

        let temp_path = self.write_temp_wav(&audio_data).await?;
        let temp_path_string = temp_path.to_string_lossy().to_string();

        let client = reqwest::Client::new();
        let result = async {
            let response = client
                .post(format!("{}/transcribe", self.base_url()))
                .json(&TranscribeRequest {
                    audio_path: &temp_path_string,
                })
                .send()
                .await
                .map_err(|e| anyhow!("Failed to call NeMo sidecar /transcribe: {}", e))?;

            if !response.status().is_success() {
                return Err(anyhow!(
                    "NeMo sidecar /transcribe failed with status {} for model {}",
                    response.status(),
                    current_model
                ));
            }

            let body: TranscribeResponse = response
                .json()
                .await
                .map_err(|e| anyhow!("Invalid NeMo sidecar /transcribe response: {}", e))?;

            Ok(body.text)
        }
        .await;

        if let Err(e) = fs::remove_file(&temp_path).await {
            log::warn!("Failed to remove temporary NeMo WAV {}: {}", temp_path.display(), e);
        }

        result
    }

    pub async fn unload_model(&self) -> bool {
        let had_model = self.current_model_name.write().await.take().is_some();

        if self.is_sidecar_healthy().await {
            let client = reqwest::Client::new();
            if let Err(e) = client.post(format!("{}/unload", self.base_url())).send().await {
                log::warn!("Failed to unload NeMo sidecar model: {}", e);
            }
        }

        had_model
    }

    pub async fn get_current_model(&self) -> Option<String> {
        self.current_model_name.read().await.clone()
    }

    pub async fn is_model_loaded(&self) -> bool {
        self.current_model_name.read().await.is_some()
    }

    pub async fn get_models_directory(&self) -> PathBuf {
        self.models_dir.clone()
    }

    pub async fn delete_model(&self, model_name: &str) -> Result<String> {
        let entry = self.catalog_entry(model_name)?;
        let model_dir = self.model_dir(entry)?;

        if model_dir.exists() {
            fs::remove_dir_all(&model_dir)
                .await
                .map_err(|e| anyhow!("Failed to delete NeMo model directory: {}", e))?;
        }

        if self.get_current_model().await.as_deref() == Some(model_name) {
            self.unload_model().await;
        }

        Ok(format!("Successfully deleted NeMo model '{}'", model_name))
    }

    pub async fn cancel_download(&self, model_name: &str) -> Result<()> {
        *self.cancel_download_flag.write().await = Some(model_name.to_string());
        self.active_downloads.write().await.remove(model_name);
        Ok(())
    }

    pub async fn download_model_detailed(
        &self,
        model_name: &str,
        progress_callback: Option<Box<dyn Fn(DownloadProgress) + Send + Sync>>,
    ) -> Result<()> {
        let entry = self.catalog_entry(model_name)?;
        let repo_id = entry
            .repo_id
            .ok_or_else(|| anyhow!("NeMo model {} does not define a Hugging Face repo", model_name))?;
        let filename = entry
            .filename
            .ok_or_else(|| anyhow!("NeMo model {} does not define a model filename", model_name))?;

        {
            let mut active = self.active_downloads.write().await;
            if !active.insert(model_name.to_string()) {
                return Err(anyhow!("Download already in progress for {}", model_name));
            }
        }

        *self.cancel_download_flag.write().await = None;

        let model_dir = self.model_dir(entry)?;
        if !model_dir.exists() {
            fs::create_dir_all(&model_dir).await?;
        }

        let file_path = model_dir.join(filename);
        let expected_size = expected_size_bytes(entry);
        if let Ok(metadata) = fs::metadata(&file_path).await {
            if metadata.len() >= expected_size {
                if let Some(callback) = progress_callback.as_ref() {
                    callback(DownloadProgress::new(expected_size, expected_size, 0.0));
                }
                self.active_downloads.write().await.remove(model_name);
                return Ok(());
            }
        }

        let url = format!("https://huggingface.co/{}/resolve/main/{}", repo_id, filename);
        let client = reqwest::Client::new();
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to start NeMo model download: {}", e))?;

        if !response.status().is_success() {
            self.active_downloads.write().await.remove(model_name);
            return Err(anyhow!("NeMo model download failed with status {}", response.status()));
        }

        let total_size = response.content_length().unwrap_or(expected_size).max(expected_size);
        let file = fs::File::create(&file_path).await?;
        let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;
        let started = Instant::now();
        let mut last_report = Instant::now();
        let mut bytes_since_report = 0u64;

        while let Some(chunk_result) = tokio::time::timeout(DOWNLOAD_STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| anyhow!("Download timeout - no data received for 30 seconds"))?
        {
            if self.cancel_download_flag.read().await.as_deref() == Some(model_name) {
                self.active_downloads.write().await.remove(model_name);
                return Err(anyhow!("Download cancelled by user"));
            }

            let chunk = chunk_result.map_err(|e| anyhow!("Download stream failed: {}", e))?;
            writer.write_all(&chunk).await?;

            let chunk_len = chunk.len() as u64;
            downloaded += chunk_len;
            bytes_since_report += chunk_len;

            let elapsed_since_report = last_report.elapsed();
            if elapsed_since_report >= Duration::from_millis(500) || downloaded >= total_size {
                let speed_mbps = if elapsed_since_report.as_secs_f64() > 0.0 {
                    (bytes_since_report as f64 / (1024.0 * 1024.0))
                        / elapsed_since_report.as_secs_f64()
                } else {
                    0.0
                };
                if let Some(callback) = progress_callback.as_ref() {
                    callback(DownloadProgress::new(downloaded, total_size, speed_mbps));
                }
                last_report = Instant::now();
                bytes_since_report = 0;
            }
        }

        writer.flush().await?;

        if let Some(callback) = progress_callback.as_ref() {
            let elapsed = started.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                (downloaded as f64 / (1024.0 * 1024.0)) / elapsed
            } else {
                0.0
            };
            callback(DownloadProgress::new(total_size, total_size, speed));
        }

        self.active_downloads.write().await.remove(model_name);
        Ok(())
    }

    fn catalog_entry(&self, model_name: &str) -> Result<&'static TranscriptionModelCatalogEntry> {
        transcription_catalog::get_transcription_model(model_name)
            .filter(|entry| entry.runtime == transcription_catalog::TranscriptionRuntime::Nemo)
            .ok_or_else(|| anyhow!("NeMo model '{}' is not in the transcription catalog", model_name))
    }

    fn model_dir(&self, entry: &TranscriptionModelCatalogEntry) -> Result<PathBuf> {
        let repo_id = entry
            .repo_id
            .ok_or_else(|| anyhow!("NeMo model {} does not define repo_id", entry.model_id))?;
        Ok(self.models_dir.join(sanitize_repo_id(repo_id)))
    }

    fn model_file_path(&self, entry: &TranscriptionModelCatalogEntry) -> Result<PathBuf> {
        let filename = entry
            .filename
            .ok_or_else(|| anyhow!("NeMo model {} does not define filename", entry.model_id))?;
        Ok(self.model_dir(entry)?.join(filename))
    }

    async fn model_status(
        &self,
        entry: &TranscriptionModelCatalogEntry,
        model_path: &Path,
    ) -> ModelStatus {
        if !model_path.exists() {
            return ModelStatus::Missing;
        }

        match fs::metadata(model_path).await {
            Ok(metadata) => {
                let expected_min_size = expected_size_bytes(entry);
                if metadata.len() >= expected_min_size {
                    ModelStatus::Available
                } else {
                    ModelStatus::Corrupted {
                        file_size: metadata.len(),
                        expected_min_size,
                    }
                }
            }
            Err(e) => ModelStatus::Error(e.to_string()),
        }
    }

    async fn ensure_sidecar_running(&self) -> Result<()> {
        if self.is_sidecar_healthy().await {
            return Ok(());
        }

        self.spawn_sidecar().await?;

        let started = Instant::now();
        while started.elapsed() < SIDECAR_STARTUP_TIMEOUT {
            if self.is_sidecar_healthy().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(anyhow!("Timed out waiting for NeMo sidecar to become healthy"))
    }

    async fn is_sidecar_healthy(&self) -> bool {
        let client = reqwest::Client::new();
        let result = client
            .get(format!("{}/health", self.base_url()))
            .timeout(Duration::from_millis(750))
            .send()
            .await;

        match result {
            Ok(response) if response.status().is_success() => {
                response
                    .json::<HealthResponse>()
                    .await
                    .map(|body| body.status == "ok")
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    async fn spawn_sidecar(&self) -> Result<()> {
        let script_path = self.find_sidecar_script()?;
        let python = std::env::var("MEETILY_NEMO_PYTHON").unwrap_or_else(|_| "python".to_string());

        let mut command = tokio::process::Command::new(python);
        command
            .arg(&script_path)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("--models-dir")
            .arg(&self.models_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn NeMo sidecar at {}: {}", script_path.display(), e))?;

        *self.child_process.lock().await = Some(child);
        Ok(())
    }

    fn find_sidecar_script(&self) -> Result<PathBuf> {
        if let Ok(path) = std::env::var("MEETILY_NEMO_SIDECAR") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Ok(path);
            }
        }

        let mut candidates = Vec::new();
        if let Some(dir) = &self.sidecar_dir {
            candidates.push(dir.join("server.py"));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            candidates.push(current_dir.join("sidecars").join("nemo_asr").join("server.py"));
            candidates.push(
                current_dir
                    .join("src-tauri")
                    .join("sidecars")
                    .join("nemo_asr")
                    .join("server.py"),
            );
            candidates.push(
                current_dir
                    .join("frontend")
                    .join("src-tauri")
                    .join("sidecars")
                    .join("nemo_asr")
                    .join("server.py"),
            );
        }

        candidates
            .into_iter()
            .find(|path| path.exists())
            .ok_or_else(|| anyhow!("NeMo sidecar script not found. Set MEETILY_NEMO_SIDECAR to server.py."))
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    async fn write_temp_wav(&self, samples: &[f32]) -> Result<PathBuf> {
        let temp_dir = self.models_dir.join("tmp");
        if !temp_dir.exists() {
            fs::create_dir_all(&temp_dir).await?;
        }

        let path = temp_dir.join(format!("nemo-{}.wav", uuid::Uuid::new_v4()));
        let samples = samples.to_vec();
        let path_for_write = path.clone();

        tokio::task::spawn_blocking(move || write_wav_16khz_mono(&path_for_write, &samples))
            .await
            .map_err(|e| anyhow!("Failed to join WAV write task: {}", e))??;

        Ok(path)
    }
}

impl Drop for NemoEngine {
    fn drop(&mut self) {
        if let Ok(mut child_guard) = self.child_process.try_lock() {
            if let Some(child) = child_guard.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

fn sanitize_repo_id(repo_id: &str) -> String {
    repo_id.replace('/', "--")
}

fn expected_size_bytes(entry: &TranscriptionModelCatalogEntry) -> u64 {
    // Allow minor upstream metadata/file-size variance while still catching
    // partial downloads.
    (entry.size_mb as u64 * 1024 * 1024 * 95) / 100
}

fn write_wav_16khz_mono(path: &Path, samples: &[f32]) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::File::create(path)?;
    let channels = 1u16;
    let sample_rate = 16_000u32;
    let bits_per_sample = 16u16;
    let bytes_per_sample = (bits_per_sample / 8) as u32;
    let data_len = samples.len() as u32 * bytes_per_sample;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample;
    let block_align = channels * (bits_per_sample / 8);

    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&bits_per_sample.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;

    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let int_sample = (clamped * i16::MAX as f32) as i16;
        file.write_all(&int_sample.to_le_bytes())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ids_are_sanitized_for_directories() {
        assert_eq!(sanitize_repo_id("nvidia/parakeet-rnnt-1.1b"), "nvidia--parakeet-rnnt-1.1b");
    }
}
