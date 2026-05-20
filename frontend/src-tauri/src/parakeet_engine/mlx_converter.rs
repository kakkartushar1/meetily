//! MLX to ONNX conversion support.
//!
//! MLX models (Apple Silicon optimized) need to be converted to ONNX format
//! for cross-platform compatibility. This module provides:
//!
//! 1. Platform detection (MLX only works on macOS + Apple Silicon)
//! 2. Conversion path via Python subprocess (mlx-onnx library)
//! 3. Safetensors → ONNX conversion pipeline
//!
//! On Windows/Linux, MLX models are flagged as requiring conversion,
//! and users are guided to use ONNX-format models instead.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Platform support status for MLX models.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MlxPlatformSupport {
    /// macOS with Apple Silicon - full MLX support
    Supported,
    /// macOS with Intel - no MLX support
    UnsupportedIntelMac,
    /// Windows - no MLX support, conversion required
    UnsupportedWindows,
    /// Linux - no MLX support, conversion required
    UnsupportedLinux,
}

impl std::fmt::Display for MlxPlatformSupport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MlxPlatformSupport::Supported => write!(f, "Supported (macOS Apple Silicon)"),
            MlxPlatformSupport::UnsupportedIntelMac => write!(f, "Unsupported (Intel Mac)"),
            MlxPlatformSupport::UnsupportedWindows => write!(f, "Unsupported (Windows)"),
            MlxPlatformSupport::UnsupportedLinux => write!(f, "Unsupported (Linux)"),
        }
    }
}

/// Conversion status for model format conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversionStatus {
    /// Not started
    Pending,
    /// Conversion in progress
    InProgress { step: String, progress: u8 },
    /// Conversion completed successfully
    Completed { output_path: PathBuf },
    /// Conversion failed
    Failed { error: String },
    /// Conversion not possible on this platform
    NotSupported { reason: String },
}

/// Check MLX platform support for the current system.
pub fn check_mlx_platform_support() -> MlxPlatformSupport {
    #[cfg(target_os = "windows")]
    {
        MlxPlatformSupport::UnsupportedWindows
    }

    #[cfg(target_os = "linux")]
    {
        MlxPlatformSupport::UnsupportedLinux
    }

    #[cfg(target_os = "macos")]
    {
        // Check for Apple Silicon
        if is_apple_silicon() {
            MlxPlatformSupport::Supported
        } else {
            MlxPlatformSupport::UnsupportedIntelMac
        }
    }
}

/// Check if the current system is Apple Silicon (ARM64 Mac).
#[cfg(target_os = "macos")]
fn is_apple_silicon() -> bool {
    std::env::consts::ARCH == "aarch64"
}

/// Get a user-friendly message about MLX model compatibility.
pub fn get_mlx_compatibility_message() -> String {
    let support = check_mlx_platform_support();
    match support {
        MlxPlatformSupport::Supported => {
            "MLX models are supported on this system. \
             They can be used directly or converted to ONNX for broader compatibility.".to_string()
        }
        MlxPlatformSupport::UnsupportedWindows => {
            "MLX models require macOS with Apple Silicon. \
             On Windows, please use ONNX-format models instead, or convert the model \
             to ONNX format on a compatible system.".to_string()
        }
        MlxPlatformSupport::UnsupportedLinux => {
            "MLX models require macOS with Apple Silicon. \
             On Linux, please use ONNX-format models instead, or convert the model \
             to ONNX format on a compatible system.".to_string()
        }
        MlxPlatformSupport::UnsupportedIntelMac => {
            "MLX models require Apple Silicon (M1/M2/M3/M4). \
             On Intel Macs, please use ONNX-format models instead.".to_string()
        }
    }
}

/// Attempt to convert an MLX model to ONNX format.
///
/// This requires:
/// - macOS with Apple Silicon
/// - Python 3.10+ installed
/// - mlx-onnx library installed (`pip install mlx-onnx`)
///
/// On unsupported platforms, returns an error with guidance.
pub async fn convert_mlx_to_onnx(
    mlx_model_path: &Path,
    output_dir: &Path,
) -> Result<ConversionStatus> {
    let support = check_mlx_platform_support();

    if support != MlxPlatformSupport::Supported {
        return Ok(ConversionStatus::NotSupported {
            reason: get_mlx_compatibility_message(),
        });
    }

    // Check if Python is available
    let python_available = check_python_available().await;
    if !python_available {
        return Ok(ConversionStatus::Failed {
            error: "Python 3.10+ is required for MLX to ONNX conversion. \
                    Please install Python and the mlx-onnx library.".to_string(),
        });
    }

    // Check if mlx-onnx is installed
    let mlx_onnx_available = check_mlx_onnx_available().await;
    if !mlx_onnx_available {
        return Ok(ConversionStatus::Failed {
            error: "The mlx-onnx library is required for conversion. \
                    Install it with: pip install mlx-onnx".to_string(),
        });
    }

    // Create output directory
    if !output_dir.exists() {
        tokio::fs::create_dir_all(output_dir).await
            .map_err(|e| anyhow!("Failed to create output directory: {}", e))?;
    }

    // Run the conversion via Python subprocess
    log::info!(
        "Starting MLX to ONNX conversion: {} -> {}",
        mlx_model_path.display(),
        output_dir.display()
    );

    let conversion_script = format!(
        r#"
import sys
try:
    from mlx_onnx import convert
    convert("{}", "{}")
    print("CONVERSION_SUCCESS")
except Exception as e:
    print(f"CONVERSION_ERROR: {{e}}")
    sys.exit(1)
"#,
        mlx_model_path.display().to_string().replace('\\', "/"),
        output_dir.display().to_string().replace('\\', "/")
    );

    let output = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(&conversion_script)
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run Python conversion: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() && stdout.contains("CONVERSION_SUCCESS") {
        log::info!("MLX to ONNX conversion completed successfully");
        Ok(ConversionStatus::Completed {
            output_path: output_dir.to_path_buf(),
        })
    } else {
        let error_msg = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        log::error!("MLX to ONNX conversion failed: {}", error_msg);
        Ok(ConversionStatus::Failed {
            error: format!("Conversion failed: {}", error_msg),
        })
    }
}

/// Attempt to convert a Safetensors model to ONNX format.
///
/// This requires Python with the `transformers` and `onnx` libraries.
pub async fn convert_safetensors_to_onnx(
    model_path: &Path,
    output_dir: &Path,
) -> Result<ConversionStatus> {
    // Check if Python is available
    let python_available = check_python_available().await;
    if !python_available {
        return Ok(ConversionStatus::Failed {
            error: "Python 3.10+ is required for Safetensors to ONNX conversion. \
                    Please install Python with the transformers and onnx libraries.".to_string(),
        });
    }

    // Create output directory
    if !output_dir.exists() {
        tokio::fs::create_dir_all(output_dir).await
            .map_err(|e| anyhow!("Failed to create output directory: {}", e))?;
    }

    log::info!(
        "Starting Safetensors to ONNX conversion: {} -> {}",
        model_path.display(),
        output_dir.display()
    );

    let conversion_script = format!(
        r#"
import sys
try:
    from transformers import AutoModel, AutoConfig
    import torch
    import os

    model_path = "{}"
    output_dir = "{}"

    # Load the model
    config = AutoConfig.from_pretrained(model_path)
    model = AutoModel.from_pretrained(model_path)
    model.eval()

    # Create dummy input based on model type
    dummy_input = torch.randn(1, 16000 * 30)  # 30 seconds of audio at 16kHz

    # Export to ONNX
    output_path = os.path.join(output_dir, "model.onnx")
    torch.onnx.export(
        model,
        dummy_input,
        output_path,
        opset_version=14,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={{
            "input": {{0: "batch_size", 1: "sequence_length"}},
            "output": {{0: "batch_size"}}
        }}
    )
    print("CONVERSION_SUCCESS")
except Exception as e:
    print(f"CONVERSION_ERROR: {{e}}")
    sys.exit(1)
"#,
        model_path.display().to_string().replace('\\', "/"),
        output_dir.display().to_string().replace('\\', "/")
    );

    let output = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(&conversion_script)
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run Python conversion: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() && stdout.contains("CONVERSION_SUCCESS") {
        log::info!("Safetensors to ONNX conversion completed successfully");
        Ok(ConversionStatus::Completed {
            output_path: output_dir.to_path_buf(),
        })
    } else {
        let error_msg = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        log::error!("Safetensors to ONNX conversion failed: {}", error_msg);
        Ok(ConversionStatus::Failed {
            error: format!("Conversion failed: {}", error_msg),
        })
    }
}

/// Check if Python 3 is available on the system.
async fn check_python_available() -> bool {
    // Try python3 first, then python
    for cmd in &["python3", "python"] {
        match tokio::process::Command::new(cmd)
            .arg("--version")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                log::info!("Found Python: {}", version.trim());
                return true;
            }
            _ => continue,
        }
    }
    false
}

/// Check if the mlx-onnx Python library is available.
async fn check_mlx_onnx_available() -> bool {
    match tokio::process::Command::new("python3")
        .args(&["-c", "import mlx_onnx; print('ok')"])
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Get information about available conversion tools.
pub fn get_conversion_capabilities() -> serde_json::Value {
    let platform = check_mlx_platform_support();

    serde_json::json!({
        "platform": format!("{}", platform),
        "mlxSupported": platform == MlxPlatformSupport::Supported,
        "conversions": {
            "mlxToOnnx": {
                "supported": platform == MlxPlatformSupport::Supported,
                "requirements": "Python 3.10+, mlx-onnx library",
                "installCommand": "pip install mlx-onnx",
            },
            "safetensorsToOnnx": {
                "supported": true, // Available on all platforms with Python
                "requirements": "Python 3.10+, transformers, torch, onnx libraries",
                "installCommand": "pip install transformers torch onnx",
            },
        },
        "recommendation": if platform == MlxPlatformSupport::Supported {
            "MLX models can be used directly or converted to ONNX."
        } else {
            "Use ONNX-format models for best compatibility on this platform."
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_support_detection() {
        let support = check_mlx_platform_support();
        // On Windows, should always be UnsupportedWindows
        #[cfg(target_os = "windows")]
        assert_eq!(support, MlxPlatformSupport::UnsupportedWindows);

        #[cfg(target_os = "linux")]
        assert_eq!(support, MlxPlatformSupport::UnsupportedLinux);
    }

    #[test]
    fn test_compatibility_message_not_empty() {
        let msg = get_mlx_compatibility_message();
        assert!(!msg.is_empty());
    }

    #[test]
    fn test_conversion_capabilities() {
        let caps = get_conversion_capabilities();
        assert!(caps.get("platform").is_some());
        assert!(caps.get("conversions").is_some());
        assert!(caps.get("recommendation").is_some());
    }

    #[test]
    fn test_platform_display() {
        assert_eq!(
            format!("{}", MlxPlatformSupport::UnsupportedWindows),
            "Unsupported (Windows)"
        );
    }
}
