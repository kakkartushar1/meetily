//! Transcription Model Catalog
//!
//! Shared catalog of all transcription models with runtime metadata.
//! This module is the single source of truth for model definitions across
//! Whisper (ggml), Parakeet (ONNX), NeMo (.nemo), and custom HuggingFace runtimes.
//!
//! The existing DB shape (provider, model) is preserved to avoid migration risk.
//! Runtime is resolved from the catalog by model name.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

// ============================================================================
// RUNTIME & PROVIDER ENUMS
// ============================================================================

/// Runtime backend used to execute a transcription model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelRuntime {
    /// ggml Whisper runtime (whisper-rs / whisper.cpp)
    LocalWhisper,
    /// ONNX Parakeet runtime (ort crate)
    Parakeet,
    /// NeMo .nemo runtime (Python sidecar)
    Nemo,
}

impl std::fmt::Display for ModelRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelRuntime::LocalWhisper => write!(f, "localWhisper"),
            ModelRuntime::Parakeet => write!(f, "parakeet"),
            ModelRuntime::Nemo => write!(f, "nemo"),
        }
    }
}

// ============================================================================
// MODEL FORMAT DETECTION
// ============================================================================

/// Detected format of a model from HuggingFace or local path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelFormat {
    /// ONNX format - ready to use with ort crate
    Onnx,
    /// MLX format - Apple Silicon optimized (safetensors with MLX config)
    Mlx,
    /// Safetensors format - PyTorch/generic, needs conversion to ONNX
    Safetensors,
    /// NeMo checkpoint format
    NemoCheckpoint,
    /// Unknown format
    Unknown,
}

impl std::fmt::Display for ModelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelFormat::Onnx => write!(f, "ONNX"),
            ModelFormat::Mlx => write!(f, "MLX"),
            ModelFormat::Safetensors => write!(f, "Safetensors"),
            ModelFormat::NemoCheckpoint => write!(f, "NeMo"),
            ModelFormat::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Status of a custom model download/conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CustomModelStatus {
    /// Model metadata fetched, not yet downloaded
    Pending,
    /// Model is being downloaded
    Downloading { progress: u8 },
    /// Model is being converted (e.g., Safetensors → ONNX)
    Converting,
    /// Model is ready to use
    Ready,
    /// Model download/conversion failed
    Error(String),
}

// ============================================================================
// CATALOG ENTRY
// ============================================================================

/// A single entry in the transcription model catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Unique model identifier used in DB `model` column.
    /// For ONNX Parakeet: e.g. "parakeet-tdt-0.6b-v3-int8"
    /// For NeMo: e.g. "nvidia/parakeet-rnnt-1.1b"
    pub model_id: &'static str,

    /// Provider string stored in DB `provider` column.
    /// Kept as "parakeet" for both ONNX and NeMo Parakeet models.
    pub provider: &'static str,

    /// Runtime backend that executes this model.
    pub runtime: ModelRuntime,

    /// HuggingFace repo ID (for downloadable models).
    pub repo: Option<&'static str>,

    /// Primary model filename within the repo or local directory.
    pub file: Option<&'static str>,

    /// Approximate download size in MB.
    pub size_mb: u32,

    /// Expected audio sample rate in Hz.
    pub sample_rate: u32,

    /// Human-readable label for UI display.
    pub label: &'static str,

    /// Short description for UI display.
    pub description: &'static str,
}

// ============================================================================
// CUSTOM MODEL CATALOG ENTRY (user-added HuggingFace models)
// ============================================================================

/// A user-added custom model from HuggingFace or local path.
/// Unlike `CatalogEntry`, this uses owned `String` fields since
/// custom models are created at runtime, not compiled in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomModelCatalogEntry {
    /// Unique model identifier (HuggingFace repo ID or local path hash).
    /// e.g. "nvidia/parakeet-tdt-0.6b-v3" or "custom-local-abc123"
    pub model_id: String,

    /// Provider string stored in DB `provider` column.
    pub provider: String,

    /// Runtime backend that executes this model.
    pub runtime: ModelRuntime,

    /// Detected model format from file inspection.
    pub format: ModelFormat,

    /// HuggingFace repo ID (if sourced from HuggingFace).
    pub repo: Option<String>,

    /// Local file path (if sourced from local filesystem).
    pub local_path: Option<PathBuf>,

    /// List of model files (filenames within the repo or directory).
    pub files: Vec<String>,

    /// Approximate download size in MB.
    pub size_mb: u32,

    /// Expected audio sample rate in Hz (default 16000).
    pub sample_rate: u32,

    /// Human-readable label for UI display.
    pub label: String,

    /// Short description for UI display.
    pub description: String,

    /// Current status of the custom model.
    pub status: CustomModelStatus,
}

impl CustomModelCatalogEntry {
    /// Create a new custom model entry from a HuggingFace repo.
    pub fn from_huggingface(
        repo_id: &str,
        format: ModelFormat,
        files: Vec<String>,
        size_mb: u32,
        label: Option<String>,
        description: Option<String>,
    ) -> Self {
        let runtime = match format {
            ModelFormat::Onnx => ModelRuntime::Parakeet,
            ModelFormat::NemoCheckpoint => ModelRuntime::Nemo,
            // MLX and Safetensors will need conversion, default to Parakeet (ONNX target)
            _ => ModelRuntime::Parakeet,
        };

        Self {
            model_id: format!("custom/{}", repo_id),
            provider: "parakeet".to_string(),
            runtime,
            format,
            repo: Some(repo_id.to_string()),
            local_path: None,
            files,
            size_mb,
            sample_rate: 16000,
            label: label.unwrap_or_else(|| repo_id.to_string()),
            description: description.unwrap_or_else(|| {
                format!("Custom HuggingFace model: {} ({})", repo_id, format)
            }),
            status: CustomModelStatus::Pending,
        }
    }

    /// Create a new custom model entry from a local path.
    pub fn from_local_path(
        path: PathBuf,
        format: ModelFormat,
        files: Vec<String>,
        size_mb: u32,
        label: Option<String>,
    ) -> Self {
        let runtime = match format {
            ModelFormat::Onnx => ModelRuntime::Parakeet,
            ModelFormat::NemoCheckpoint => ModelRuntime::Nemo,
            _ => ModelRuntime::Parakeet,
        };

        let path_display = path.display().to_string();
        Self {
            model_id: format!("local/{}", path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())),
            provider: "parakeet".to_string(),
            runtime,
            format,
            repo: None,
            local_path: Some(path),
            files,
            size_mb,
            sample_rate: 16000,
            label: label.unwrap_or_else(|| path_display.clone()),
            description: format!("Local model: {} ({})", path_display, format),
            status: CustomModelStatus::Pending,
        }
    }

    /// Check if this model is ready for transcription.
    pub fn is_ready(&self) -> bool {
        self.status == CustomModelStatus::Ready
    }

    /// Check if this model needs format conversion before use.
    pub fn needs_conversion(&self) -> bool {
        matches!(self.format, ModelFormat::Mlx | ModelFormat::Safetensors)
    }
}

// ============================================================================
// STATIC CATALOG
// ============================================================================

/// Complete catalog of all known transcription models.
///
/// Whisper models are defined separately in `config::WHISPER_MODEL_CATALOG`
/// because they follow a different metadata shape (filename, accuracy, speed).
/// This catalog focuses on Parakeet-family models (ONNX + NeMo).
pub static PARAKEET_MODEL_CATALOG: &[CatalogEntry] = &[
    // ── ONNX Parakeet models (existing) ──────────────────────────────────
    CatalogEntry {
        model_id: "parakeet-tdt-0.6b-v3-int8",
        provider: "parakeet",
        runtime: ModelRuntime::Parakeet,
        repo: None, // downloaded from custom CDN, not HF
        file: None,
        size_mb: 670,
        sample_rate: 16000,
        label: "Parakeet TDT 0.6B v3 (Int8)",
        description: "Ultra Fast – real time on M4 Max, latest version with int8 quantization",
    },
    CatalogEntry {
        model_id: "parakeet-tdt-0.6b-v2-int8",
        provider: "parakeet",
        runtime: ModelRuntime::Parakeet,
        repo: None,
        file: None,
        size_mb: 661,
        sample_rate: 16000,
        label: "Parakeet TDT 0.6B v2 (Int8)",
        description: "Fast – previous version with int8 quantization, good balance of speed and accuracy",
    },
    // ── NeMo Parakeet models (new) ───────────────────────────────────────
    CatalogEntry {
        model_id: "nvidia/parakeet-rnnt-1.1b",
        provider: "parakeet",
        runtime: ModelRuntime::Nemo,
        repo: Some("nvidia/parakeet-rnnt-1.1b"),
        file: Some("parakeet-rnnt-1.1b.nemo"),
        size_mb: 4280,
        sample_rate: 16000,
        label: "Parakeet RNNT 1.1B",
        description: "High-accuracy English ASR – opt-in download, requires ~4.3 GB",
    },
];

// ============================================================================
// CUSTOM MODEL REGISTRY (runtime storage)
// ============================================================================

lazy_static::lazy_static! {
    /// Global registry of user-added custom models.
    /// Thread-safe read-write access for runtime model management.
    static ref CUSTOM_MODEL_REGISTRY: RwLock<Vec<CustomModelCatalogEntry>> = RwLock::new(Vec::new());
}

// ============================================================================
// LOOKUP HELPERS
// ============================================================================

/// Look up a catalog entry by model_id.
pub fn lookup_model(model_id: &str) -> Option<&'static CatalogEntry> {
    PARAKEET_MODEL_CATALOG.iter().find(|e| e.model_id == model_id)
}

/// Look up a custom model by model_id.
pub fn lookup_custom_model(model_id: &str) -> Option<CustomModelCatalogEntry> {
    let registry = CUSTOM_MODEL_REGISTRY.read().ok()?;
    registry.iter().find(|e| e.model_id == model_id).cloned()
}

/// Resolve the runtime for a given (provider, model) pair.
///
/// Returns `None` if the model is not in any catalog
/// (e.g. it's a Whisper model handled by the Whisper engine).
pub fn resolve_runtime(provider: &str, model_id: &str) -> Option<ModelRuntime> {
    // Whisper models are never in the Parakeet catalog
    if provider == "localWhisper" || provider == "whisper" {
        return Some(ModelRuntime::LocalWhisper);
    }

    // Check built-in catalog first
    if let Some(entry) = lookup_model(model_id) {
        return Some(entry.runtime);
    }

    // Check custom model registry
    if let Some(custom) = lookup_custom_model(model_id) {
        return Some(custom.runtime);
    }

    None
}

/// Check whether a model_id refers to a NeMo runtime model.
pub fn is_nemo_model(model_id: &str) -> bool {
    matches!(resolve_runtime("parakeet", model_id), Some(ModelRuntime::Nemo))
}

/// Check whether a model_id refers to a custom (user-added) model.
pub fn is_custom_model(model_id: &str) -> bool {
    model_id.starts_with("custom/") || model_id.starts_with("local/")
}

/// Get all catalog entries for a specific runtime.
pub fn models_for_runtime(runtime: ModelRuntime) -> Vec<&'static CatalogEntry> {
    PARAKEET_MODEL_CATALOG
        .iter()
        .filter(|e| e.runtime == runtime)
        .collect()
}

// ============================================================================
// CUSTOM MODEL REGISTRY MANAGEMENT
// ============================================================================

/// Register a new custom model in the runtime registry.
pub fn register_custom_model(entry: CustomModelCatalogEntry) -> Result<(), String> {
    let mut registry = CUSTOM_MODEL_REGISTRY.write()
        .map_err(|e| format!("Failed to acquire write lock on custom model registry: {}", e))?;

    // Check for duplicate model_id
    if registry.iter().any(|e| e.model_id == entry.model_id) {
        return Err(format!("Custom model '{}' is already registered", entry.model_id));
    }

    log::info!("Registering custom model: {} (format: {}, runtime: {})",
        entry.model_id, entry.format, entry.runtime);
    registry.push(entry);
    Ok(())
}

/// Remove a custom model from the registry.
pub fn unregister_custom_model(model_id: &str) -> Result<CustomModelCatalogEntry, String> {
    let mut registry = CUSTOM_MODEL_REGISTRY.write()
        .map_err(|e| format!("Failed to acquire write lock on custom model registry: {}", e))?;

    let idx = registry.iter().position(|e| e.model_id == model_id)
        .ok_or_else(|| format!("Custom model '{}' not found in registry", model_id))?;

    log::info!("Unregistering custom model: {}", model_id);
    Ok(registry.remove(idx))
}

/// Update the status of a custom model.
pub fn update_custom_model_status(model_id: &str, status: CustomModelStatus) -> Result<(), String> {
    let mut registry = CUSTOM_MODEL_REGISTRY.write()
        .map_err(|e| format!("Failed to acquire write lock on custom model registry: {}", e))?;

    let entry = registry.iter_mut().find(|e| e.model_id == model_id)
        .ok_or_else(|| format!("Custom model '{}' not found in registry", model_id))?;

    log::info!("Updating custom model '{}' status to: {:?}", model_id, status);
    entry.status = status;
    Ok(())
}

/// Get all registered custom models.
pub fn get_custom_models() -> Vec<CustomModelCatalogEntry> {
    CUSTOM_MODEL_REGISTRY.read()
        .map(|registry| registry.clone())
        .unwrap_or_default()
}

/// Get all custom models that are ready for transcription.
pub fn get_ready_custom_models() -> Vec<CustomModelCatalogEntry> {
    get_custom_models().into_iter().filter(|m| m.is_ready()).collect()
}

// ============================================================================
// FORMAT DETECTION
// ============================================================================

/// Detect the model format from a list of filenames.
///
/// Inspects file extensions and known config files to determine
/// the model format (ONNX, MLX, Safetensors, NeMo).
pub fn detect_model_format(files: &[String]) -> ModelFormat {
    let has_onnx = files.iter().any(|f| f.ends_with(".onnx"));
    let has_safetensors = files.iter().any(|f| f.ends_with(".safetensors"));
    let has_nemo = files.iter().any(|f| f.ends_with(".nemo"));
    let has_mlx_config = files.iter().any(|f| {
        f == "mlx_config.json" || f.contains("mlx") || f == "config.json"
    });
    let has_mlx_weights = files.iter().any(|f| {
        f == "weights.safetensors" || f == "model.safetensors"
    });

    // Priority: ONNX > NeMo > MLX > Safetensors > Unknown
    if has_onnx {
        ModelFormat::Onnx
    } else if has_nemo {
        ModelFormat::NemoCheckpoint
    } else if has_mlx_config && has_mlx_weights {
        // MLX models typically have config.json + weights.safetensors
        // with specific MLX-related config keys
        ModelFormat::Mlx
    } else if has_safetensors {
        ModelFormat::Safetensors
    } else {
        ModelFormat::Unknown
    }
}

/// Detect model format from a local directory path.
pub fn detect_model_format_from_path(path: &std::path::Path) -> ModelFormat {
    if !path.exists() || !path.is_dir() {
        return ModelFormat::Unknown;
    }

    let files: Vec<String> = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => return ModelFormat::Unknown,
    };

    detect_model_format(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_onnx_model_resolves_to_parakeet_runtime() {
        let runtime = resolve_runtime("parakeet", "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(runtime, Some(ModelRuntime::Parakeet));
    }

    #[test]
    fn test_nemo_model_resolves_to_nemo_runtime() {
        let runtime = resolve_runtime("parakeet", "nvidia/parakeet-rnnt-1.1b");
        assert_eq!(runtime, Some(ModelRuntime::Nemo));
    }

    #[test]
    fn test_whisper_resolves_to_local_whisper_runtime() {
        let runtime = resolve_runtime("localWhisper", "large-v3-turbo");
        assert_eq!(runtime, Some(ModelRuntime::LocalWhisper));
    }

    #[test]
    fn test_unknown_model_returns_none() {
        let runtime = resolve_runtime("parakeet", "nonexistent-model");
        assert_eq!(runtime, None);
    }

    #[test]
    fn test_is_nemo_model() {
        assert!(is_nemo_model("nvidia/parakeet-rnnt-1.1b"));
        assert!(!is_nemo_model("parakeet-tdt-0.6b-v3-int8"));
    }

    #[test]
    fn test_lookup_model() {
        let entry = lookup_model("nvidia/parakeet-rnnt-1.1b").unwrap();
        assert_eq!(entry.provider, "parakeet");
        assert_eq!(entry.runtime, ModelRuntime::Nemo);
        assert_eq!(entry.size_mb, 4280);
        assert_eq!(entry.sample_rate, 16000);
    }

    #[test]
    fn test_models_for_runtime() {
        let onnx_models = models_for_runtime(ModelRuntime::Parakeet);
        assert_eq!(onnx_models.len(), 2);

        let nemo_models = models_for_runtime(ModelRuntime::Nemo);
        assert_eq!(nemo_models.len(), 1);
        assert_eq!(nemo_models[0].model_id, "nvidia/parakeet-rnnt-1.1b");
    }

    #[test]
    fn test_default_model_is_in_catalog() {
        let entry = lookup_model(crate::config::DEFAULT_PARAKEET_MODEL);
        assert!(entry.is_some(), "Default parakeet model must be in catalog");
        assert_eq!(entry.unwrap().runtime, ModelRuntime::Parakeet);
    }

    // ── Format detection tests ────────────────────────────────────────────

    #[test]
    fn test_detect_onnx_format() {
        let files = vec![
            "encoder-model.int8.onnx".to_string(),
            "decoder_joint-model.int8.onnx".to_string(),
            "vocab.txt".to_string(),
        ];
        assert_eq!(detect_model_format(&files), ModelFormat::Onnx);
    }

    #[test]
    fn test_detect_mlx_format() {
        let files = vec![
            "config.json".to_string(),
            "weights.safetensors".to_string(),
            "tokenizer.json".to_string(),
        ];
        assert_eq!(detect_model_format(&files), ModelFormat::Mlx);
    }

    #[test]
    fn test_detect_safetensors_format() {
        let files = vec![
            "pytorch_model.safetensors".to_string(),
            "tokenizer.json".to_string(),
        ];
        assert_eq!(detect_model_format(&files), ModelFormat::Safetensors);
    }

    #[test]
    fn test_detect_nemo_format() {
        let files = vec![
            "parakeet-rnnt-1.1b.nemo".to_string(),
        ];
        assert_eq!(detect_model_format(&files), ModelFormat::NemoCheckpoint);
    }

    #[test]
    fn test_detect_unknown_format() {
        let files = vec!["README.md".to_string(), "LICENSE".to_string()];
        assert_eq!(detect_model_format(&files), ModelFormat::Unknown);
    }

    #[test]
    fn test_onnx_takes_priority_over_safetensors() {
        let files = vec![
            "model.onnx".to_string(),
            "model.safetensors".to_string(),
        ];
        assert_eq!(detect_model_format(&files), ModelFormat::Onnx);
    }

    // ── Custom model tests ───────────────────────────────────────────────

    #[test]
    fn test_is_custom_model() {
        assert!(is_custom_model("custom/nvidia/parakeet-tdt-0.6b"));
        assert!(is_custom_model("local/my-model"));
        assert!(!is_custom_model("parakeet-tdt-0.6b-v3-int8"));
    }

    #[test]
    fn test_custom_model_from_huggingface() {
        let entry = CustomModelCatalogEntry::from_huggingface(
            "nvidia/parakeet-tdt-0.6b",
            ModelFormat::Onnx,
            vec!["encoder.onnx".to_string(), "decoder.onnx".to_string()],
            650,
            Some("My Custom Parakeet".to_string()),
            None,
        );

        assert_eq!(entry.model_id, "custom/nvidia/parakeet-tdt-0.6b");
        assert_eq!(entry.runtime, ModelRuntime::Parakeet);
        assert_eq!(entry.format, ModelFormat::Onnx);
        assert!(!entry.is_ready());
        assert!(!entry.needs_conversion());
    }

    #[test]
    fn test_custom_model_needs_conversion() {
        let entry = CustomModelCatalogEntry::from_huggingface(
            "some/mlx-model",
            ModelFormat::Mlx,
            vec!["weights.safetensors".to_string()],
            300,
            None,
            None,
        );

        assert!(entry.needs_conversion());
    }

    #[test]
    fn test_model_format_display() {
        assert_eq!(format!("{}", ModelFormat::Onnx), "ONNX");
        assert_eq!(format!("{}", ModelFormat::Mlx), "MLX");
        assert_eq!(format!("{}", ModelFormat::Safetensors), "Safetensors");
        assert_eq!(format!("{}", ModelFormat::NemoCheckpoint), "NeMo");
        assert_eq!(format!("{}", ModelFormat::Unknown), "Unknown");
    }
}
