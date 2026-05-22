//! HuggingFace API integration for model discovery and download.
//!
//! Provides functions to query the HuggingFace Hub API for model metadata,
//! list model files, detect model formats, and download model files with
//! progress tracking.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::model_catalog::{ModelFormat, detect_model_format};

// ============================================================================
// HUGGINGFACE API TYPES
// ============================================================================

/// A file entry from the HuggingFace Hub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceFileEntry {
    /// Relative path of the file within the repo (e.g., "encoder-model.onnx")
    #[serde(rename = "rfilename")]
    pub filename: String,

    /// File size in bytes
    #[serde(default)]
    pub size: u64,

    /// Blob ID (SHA256 hash)
    #[serde(default)]
    pub blob_id: Option<String>,

    /// LFS info if the file is stored in Git LFS
    #[serde(default)]
    pub lfs: Option<LfsInfo>,
}

/// LFS (Large File Storage) metadata for a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LfsInfo {
    /// SHA256 hash of the file content
    pub sha256: Option<String>,
    /// File size in bytes
    pub size: u64,
}

/// Model info from the HuggingFace Hub API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceModelInfo {
    /// Model ID (e.g., "nvidia/parakeet-tdt-0.6b")
    #[serde(rename = "modelId", alias = "id")]
    pub model_id: String,

    /// Author/organization
    #[serde(default)]
    pub author: Option<String>,

    /// Model tags
    #[serde(default)]
    pub tags: Vec<String>,

    /// Pipeline tag (e.g., "automatic-speech-recognition")
    #[serde(default, rename = "pipeline_tag")]
    pub pipeline_tag: Option<String>,

    /// Number of downloads
    #[serde(default)]
    pub downloads: u64,

    /// Number of likes
    #[serde(default)]
    pub likes: u64,
}

/// Result of inspecting a HuggingFace model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInspectionResult {
    /// Model info from the API
    pub model_id: String,
    /// List of files in the repo
    pub files: Vec<HuggingFaceFileEntry>,
    /// Detected model format
    pub format: ModelFormat,
    /// Total size of all files in MB
    pub total_size_mb: u32,
    /// Files relevant to the model (ONNX, safetensors, etc.)
    pub model_files: Vec<String>,
    /// Whether the model is an ASR/STT model
    pub is_stt_model: bool,
}

/// Download progress for HuggingFace model downloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfDownloadProgress {
    /// Current file being downloaded
    pub current_file: String,
    /// File index (1-based)
    pub file_index: usize,
    /// Total number of files
    pub total_files: usize,
    /// Bytes downloaded for current file
    pub downloaded_bytes: u64,
    /// Total bytes for current file
    pub total_bytes: u64,
    /// Overall progress percentage (0-100)
    pub overall_percent: u8,
    /// Download speed in MB/s
    pub speed_mbps: f64,
}

// ============================================================================
// HUGGINGFACE API CLIENT
// ============================================================================

const HF_API_BASE: &str = "https://huggingface.co/api/models";
const HF_CDN_BASE: &str = "https://huggingface.co";

/// List all files in a HuggingFace model repository.
///
/// # Arguments
/// * `repo_id` - The HuggingFace repo ID (e.g., "nvidia/parakeet-tdt-0.6b-v3")
///
/// # Returns
/// A list of file entries with their metadata.
pub async fn list_model_files(repo_id: &str) -> Result<Vec<HuggingFaceFileEntry>> {
    let url = format!("{}/{}", HF_API_BASE, repo_id);

    log::info!("Fetching model info from HuggingFace: {}", url);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    // First get model info to check if it exists
    let response = client
        .get(&url)
        .header("User-Agent", "Meetily/0.3.1")
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch model info: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "HuggingFace API returned {}: {}. Check that '{}' is a valid public model.",
            status, body, repo_id
        ));
    }

    // Now fetch the file tree
    let tree_url = format!("{}/{}/tree/main", HF_API_BASE, repo_id);
    log::info!("Fetching file tree from: {}", tree_url);

    let tree_response = client
        .get(&tree_url)
        .header("User-Agent", "Meetily/0.3.1")
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch file tree: {}", e))?;

    if !tree_response.status().is_success() {
        let status = tree_response.status();
        return Err(anyhow!("Failed to fetch file tree: HTTP {}", status));
    }

    let files: Vec<HuggingFaceFileEntry> = tree_response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse file tree response: {}", e))?;

    log::info!("Found {} files in repo '{}'", files.len(), repo_id);
    Ok(files)
}

/// Get model info from HuggingFace Hub.
pub async fn get_model_info(repo_id: &str) -> Result<HuggingFaceModelInfo> {
    let url = format!("{}/{}", HF_API_BASE, repo_id);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("User-Agent", "Meetily/0.3.1")
        .send()
        .await
        .map_err(|e| anyhow!("Failed to fetch model info: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(anyhow!("HuggingFace API returned {}: Model '{}' not found", status, repo_id));
    }

    let info: HuggingFaceModelInfo = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse model info: {}", e))?;

    Ok(info)
}

/// Inspect a HuggingFace model to determine its format, size, and compatibility.
///
/// This combines listing files and detecting format into a single operation.
pub async fn inspect_model(repo_id: &str) -> Result<ModelInspectionResult> {
    log::info!("Inspecting HuggingFace model: {}", repo_id);

    // Get model info for metadata
    let model_info = get_model_info(repo_id).await?;

    // Get file list
    let files = list_model_files(repo_id).await?;

    // Extract filenames for format detection
    let filenames: Vec<String> = files.iter().map(|f| f.filename.clone()).collect();

    // Detect format
    let format = detect_model_format(&filenames);

    // Calculate total size
    let total_size_bytes: u64 = files.iter().map(|f| {
        // Use LFS size if available, otherwise use regular size
        f.lfs.as_ref().map(|lfs| lfs.size).unwrap_or(f.size)
    }).sum();
    let total_size_mb = (total_size_bytes / (1024 * 1024)) as u32;

    // Filter model-relevant files
    let model_files: Vec<String> = filenames.iter().filter(|f| {
        f.ends_with(".onnx")
            || f.ends_with(".safetensors")
            || f.ends_with(".nemo")
            || f.ends_with(".bin")
            || f == &"vocab.txt"
            || f == &"tokenizer.json"
            || f == &"config.json"
            || f.ends_with(".onnx.data")
    }).cloned().collect();

    // Check if it's an STT model
    let is_stt_model = model_info.pipeline_tag
        .as_ref()
        .map(|tag| {
            tag == "automatic-speech-recognition"
                || tag == "speech-recognition"
                || tag == "audio-to-text"
        })
        .unwrap_or(false)
        || model_info.tags.iter().any(|t| {
            t.contains("speech") || t.contains("asr") || t.contains("stt")
                || t.contains("transcription") || t.contains("parakeet")
                || t.contains("whisper") || t.contains("nemo")
        });

    let result = ModelInspectionResult {
        model_id: repo_id.to_string(),
        files,
        format,
        total_size_mb,
        model_files,
        is_stt_model,
    };

    log::info!(
        "Model '{}' inspection: format={}, size={}MB, is_stt={}, model_files={}",
        repo_id, format, total_size_mb, is_stt_model, result.model_files.len()
    );

    Ok(result)
}

// ============================================================================
// HUGGINGFACE MODEL DOWNLOAD
// ============================================================================

/// Download a single file from a HuggingFace repository.
///
/// # Arguments
/// * `repo_id` - The HuggingFace repo ID
/// * `filename` - The file to download (relative path in repo)
/// * `dest_dir` - Local directory to save the file
/// * `progress_callback` - Optional callback for download progress
///
/// # Returns
/// The local path to the downloaded file.
pub async fn download_file(
    repo_id: &str,
    filename: &str,
    dest_dir: &Path,
    progress_callback: Option<&(dyn Fn(u64, u64, f64) + Send + Sync)>,
) -> Result<PathBuf> {
    let download_url = format!(
        "{}/{}/resolve/main/{}",
        HF_CDN_BASE, repo_id, filename
    );

    log::info!("Downloading {} from {}", filename, download_url);

    // Create destination directory
    if !dest_dir.exists() {
        fs::create_dir_all(dest_dir).await
            .map_err(|e| anyhow!("Failed to create directory {}: {}", dest_dir.display(), e))?;
    }

    // Handle nested paths (e.g., "subfolder/file.onnx")
    let dest_path = dest_dir.join(filename);
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).await
                .map_err(|e| anyhow!("Failed to create parent directory: {}", e))?;
        }
    }

    let client = reqwest::Client::builder()
        .tcp_nodelay(true)
        .timeout(Duration::from_secs(3600))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

    // Check for existing partial download (resume support)
    let mut downloaded_bytes: u64 = 0;
    let mut request = client
        .get(&download_url)
        .header("User-Agent", "Meetily/0.3.1");

    if dest_path.exists() {
        let existing_size = fs::metadata(&dest_path).await
            .map(|m| m.len())
            .unwrap_or(0);
        if existing_size > 0 {
            log::info!("Resuming download from {} bytes for {}", existing_size, filename);
            request = request.header("Range", format!("bytes={}-", existing_size));
            downloaded_bytes = existing_size;
        }
    }

    let response = request.send().await
        .map_err(|e| anyhow!("Failed to start download of {}: {}", filename, e))?;

    if !response.status().is_success() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(anyhow!(
            "Failed to download {}: HTTP {}",
            filename, response.status()
        ));
    }

    let total_bytes = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        // Resumed download: total = content-range total
        response.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split('/').last())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(downloaded_bytes + response.content_length().unwrap_or(0))
    } else {
        response.content_length().unwrap_or(0)
    };

    // Open file for writing (append if resuming)
    let file = if downloaded_bytes > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&dest_path)
            .await
            .map_err(|e| anyhow!("Failed to open file for resume: {}", e))?
    } else {
        tokio::fs::File::create(&dest_path)
            .await
            .map_err(|e| anyhow!("Failed to create file {}: {}", dest_path.display(), e))?
    };

    let mut writer = tokio::io::BufWriter::new(file);
    let mut stream = response.bytes_stream();
    let start_time = Instant::now();
    let mut last_progress_time = Instant::now();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("Download stream error: {}", e))?;
        writer.write_all(&chunk).await
            .map_err(|e| anyhow!("Failed to write chunk: {}", e))?;

        downloaded_bytes += chunk.len() as u64;

        // Report progress every 200ms
        if last_progress_time.elapsed() > Duration::from_millis(200) {
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_mbps = if elapsed > 0.0 {
                (downloaded_bytes as f64 / (1024.0 * 1024.0)) / elapsed
            } else {
                0.0
            };

            if let Some(cb) = &progress_callback {
                cb(downloaded_bytes, total_bytes, speed_mbps);
            }

            last_progress_time = Instant::now();
        }
    }

    writer.flush().await
        .map_err(|e| anyhow!("Failed to flush file: {}", e))?;

    log::info!("Successfully downloaded {} ({} bytes)", filename, downloaded_bytes);
    Ok(dest_path)
}

/// Download all model files from a HuggingFace repository.
///
/// # Arguments
/// * `repo_id` - The HuggingFace repo ID
/// * `files` - List of files to download
/// * `dest_dir` - Local directory to save files
/// * `progress_callback` - Optional callback for overall download progress
pub async fn download_model_files(
    repo_id: &str,
    files: &[String],
    dest_dir: &Path,
    progress_callback: Option<Box<dyn Fn(HfDownloadProgress) + Send + Sync>>,
) -> Result<Vec<PathBuf>> {
    log::info!(
        "Downloading {} files from '{}' to '{}'",
        files.len(), repo_id, dest_dir.display()
    );

    let total_files = files.len();
    let mut downloaded_paths = Vec::with_capacity(total_files);
    let progress_callback = progress_callback.map(std::sync::Arc::new);

    for (idx, filename) in files.iter().enumerate() {
        let file_index = idx + 1;

        log::info!("Downloading file {}/{}: {}", file_index, total_files, filename);

        let file_progress: Box<dyn Fn(u64, u64, f64) + Send + Sync> = if let Some(ref cb) = progress_callback {
            let cb = cb.clone();
            let fname = filename.clone();
            Box::new(move |downloaded: u64, total: u64, speed: f64| {
                let file_percent = if total > 0 {
                    (downloaded as f64 / total as f64 * 100.0) as u8
                } else {
                    0
                };

                // Overall progress: completed files + current file progress
                let overall = ((idx as f64 + file_percent as f64 / 100.0) / total_files as f64 * 100.0) as u8;

                cb(HfDownloadProgress {
                    current_file: fname.clone(),
                    file_index,
                    total_files,
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                    overall_percent: overall.min(100),
                    speed_mbps: speed,
                });
            })
        } else {
            Box::new(|_: u64, _: u64, _: f64| {})
        };

        let path = download_file(
            repo_id,
            filename,
            dest_dir,
            Some(&*file_progress),
        ).await?;

        downloaded_paths.push(path);
    }

    log::info!("All {} files downloaded successfully", total_files);
    Ok(downloaded_paths)
}

/// Validate a local model directory has the expected files.
pub async fn validate_local_model_path(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Err(anyhow!("Path does not exist: {}", path.display()));
    }

    if !path.is_dir() {
        return Err(anyhow!("Path is not a directory: {}", path.display()));
    }

    let mut files = Vec::new();
    let mut entries = fs::read_dir(path).await
        .map_err(|e| anyhow!("Failed to read directory: {}", e))?;

    while let Some(entry) = entries.next_entry().await
        .map_err(|e| anyhow!("Failed to read directory entry: {}", e))?
    {
        if entry.path().is_file() {
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
    }

    if files.is_empty() {
        return Err(anyhow!("Directory is empty: {}", path.display()));
    }

    let format = detect_model_format(&files);
    if format == ModelFormat::Unknown {
        return Err(anyhow!(
            "No recognized model files found in {}. Expected ONNX, Safetensors, MLX, or NeMo files.",
            path.display()
        ));
    }

    log::info!("Local model path validated: {} files, format: {}", files.len(), format);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_inspection_result_serialization() {
        let result = ModelInspectionResult {
            model_id: "test/model".to_string(),
            files: vec![],
            format: ModelFormat::Onnx,
            total_size_mb: 650,
            model_files: vec!["encoder.onnx".to_string()],
            is_stt_model: true,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("test/model"));
        assert!(json.contains("onnx"));
    }

    #[test]
    fn test_hf_download_progress() {
        let progress = HfDownloadProgress {
            current_file: "encoder.onnx".to_string(),
            file_index: 1,
            total_files: 3,
            downloaded_bytes: 100_000_000,
            total_bytes: 650_000_000,
            overall_percent: 15,
            speed_mbps: 25.5,
        };

        assert_eq!(progress.file_index, 1);
        assert_eq!(progress.overall_percent, 15);
    }
}
