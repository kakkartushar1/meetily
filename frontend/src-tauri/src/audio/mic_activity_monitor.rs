//! Microphone Activity Monitor
//!
//! Monitors the default microphone for sustained audio activity to detect
//! when a meeting/call may have started. When activity is detected, emits
//! a Tauri event so the frontend can show a toast notification prompting
//! the user to start recording.
//!
//! Key design decisions:
//! - Uses a separate low-overhead cpal stream (not the recording pipeline)
//! - Checks RMS level against a configurable threshold
//! - Requires sustained activity (configurable duration) to avoid false positives
//! - Automatically pauses monitoring while a recording is in progress
//! - Preference is persisted via tauri-plugin-store

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;

// ============================================================================
// CONFIGURATION
// ============================================================================

/// RMS threshold (0.0–1.0) above which we consider the mic "active".
/// A typical speaking voice into a close mic produces RMS ~0.01–0.05.
const DEFAULT_RMS_THRESHOLD: f32 = 0.008;

/// How long (seconds) the mic must stay above threshold before we fire
/// the "meeting detected" event.  Avoids transient noises.
const DEFAULT_SUSTAINED_SECONDS: u64 = 3;

/// Cooldown after a detection event before we can fire again (seconds).
/// Prevents spamming the user with repeated notifications.
const DETECTION_COOLDOWN_SECONDS: u64 = 60;

/// How often we evaluate the accumulated samples (milliseconds).
const EVALUATION_INTERVAL_MS: u64 = 250;

// ============================================================================
// TYPES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicActivityEvent {
    pub detected: bool,
    pub rms_level: f32,
    pub device_name: String,
    pub timestamp: u64,
}

// ============================================================================
// GLOBAL STATE
// ============================================================================

static IS_MONITORING: AtomicBool = AtomicBool::new(false);
static MEETING_DETECTED: AtomicBool = AtomicBool::new(false);

/// Shared buffer for samples coming from the cpal callback (audio thread).
/// We accumulate samples here and periodically evaluate RMS in a tokio task.
static SAMPLE_BUFFER: std::sync::LazyLock<Mutex<Vec<f32>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::with_capacity(4800))); // ~100ms at 48kHz

/// Handle to the cpal stream so we can stop it.
static STREAM_HANDLE: std::sync::LazyLock<Mutex<Option<cpal::Stream>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Name of the device currently being monitored.
static MONITORED_DEVICE: std::sync::LazyLock<Mutex<String>> =
    std::sync::LazyLock::new(|| Mutex::new(String::new()));

// ============================================================================
// PREFERENCE HELPERS
// ============================================================================

const STORE_FILE: &str = "preferences.json";
const STORE_KEY: &str = "mic_activity_monitoring_enabled";

/// Load the user's preference for mic-activity monitoring.
pub async fn load_preference<R: Runtime>(app: &AppHandle<R>) -> bool {
    match app.store(STORE_FILE) {
        Ok(store) => store
            .get(STORE_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(false), // default OFF
        Err(_) => false,
    }
}

/// Save the user's preference for mic-activity monitoring.
pub async fn save_preference<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), String> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| format!("Failed to open store: {}", e))?;
    store.set(STORE_KEY, serde_json::json!(enabled));
    store
        .save()
        .map_err(|e| format!("Failed to persist store: {}", e))?;
    Ok(())
}

// ============================================================================
// CORE MONITORING LOGIC
// ============================================================================

/// Start monitoring the default input device for sustained mic activity.
///
/// This opens a lightweight cpal input stream that feeds samples into a
/// shared buffer.  A tokio task periodically evaluates the RMS and, when
/// sustained activity is detected, emits `mic-activity-detected`.
pub async fn start_monitoring<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    // Prevent double-start
    if IS_MONITORING.load(Ordering::SeqCst) {
        info!("Mic activity monitor is already running");
        return Ok(());
    }

    info!("🎙️ Starting mic activity monitor...");

    // ── Open the default input device ──────────────────────────────────
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default input device available".to_string())?;

    let device_name = device.name().unwrap_or_else(|_| "Unknown".into());
    info!("Mic activity monitor using device: {}", device_name);

    {
        let mut name = MONITORED_DEVICE.lock().unwrap();
        *name = device_name.clone();
    }

    let config = device
        .default_input_config()
        .map_err(|e| format!("Failed to get default input config: {}", e))?;

    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    // ── Build the cpal stream ─────────────────────────────────────────
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = SAMPLE_BUFFER.lock() {
                        // Keep buffer bounded – drop old samples if behind
                        if buf.len() > 96_000 {
                            buf.drain(..buf.len() - 48_000);
                        }
                        buf.extend_from_slice(data);
                    }
                },
                |err| error!("Mic activity monitor stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {}", e))?,
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = SAMPLE_BUFFER.lock() {
                        if buf.len() > 96_000 {
                            buf.drain(..buf.len() - 48_000);
                        }
                        for &s in data {
                            buf.push(s as f32 / i16::MAX as f32);
                        }
                    }
                },
                |err| error!("Mic activity monitor stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream (i16): {}", e))?,
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = SAMPLE_BUFFER.lock() {
                        if buf.len() > 96_000 {
                            buf.drain(..buf.len() - 48_000);
                        }
                        for &s in data {
                            buf.push((s as f32 / u16::MAX as f32) * 2.0 - 1.0);
                        }
                    }
                },
                |err| error!("Mic activity monitor stream error: {}", err),
                None,
            )
            .map_err(|e| format!("Failed to build input stream (u16): {}", e))?,
        _ => {
            return Err(format!(
                "Unsupported sample format: {:?}",
                sample_format
            ));
        }
    };

    stream
        .play()
        .map_err(|e| format!("Failed to start input stream: {}", e))?;

    // Store stream handle for later cleanup
    {
        let mut handle = STREAM_HANDLE.lock().unwrap();
        *handle = Some(stream);
    }

    IS_MONITORING.store(true, Ordering::SeqCst);
    MEETING_DETECTED.store(false, Ordering::SeqCst);

    // ── Spawn evaluation task ─────────────────────────────────────────
    let app_clone = app.clone();
    tokio::spawn(async move {
        let mut activity_start: Option<Instant> = None;
        let mut last_detection: Option<Instant> = None;
        let sustained_duration = Duration::from_secs(DEFAULT_SUSTAINED_SECONDS);
        let cooldown = Duration::from_secs(DETECTION_COOLDOWN_SECONDS);
        let eval_interval = Duration::from_millis(EVALUATION_INTERVAL_MS);

        while IS_MONITORING.load(Ordering::SeqCst) {
            tokio::time::sleep(eval_interval).await;

            // Skip evaluation while a recording is in progress
            if super::recording_commands::is_recording().await {
                activity_start = None;
                continue;
            }

            // Drain the sample buffer and compute RMS
            let rms = {
                let mut buf = match SAMPLE_BUFFER.lock() {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if buf.is_empty() {
                    0.0_f32
                } else {
                    let sum_sq: f32 = buf.iter().map(|s| s * s).sum();
                    let rms = (sum_sq / buf.len() as f32).sqrt();
                    buf.clear();
                    rms
                }
            };

            let is_active = rms > DEFAULT_RMS_THRESHOLD;

            if is_active {
                match activity_start {
                    None => {
                        activity_start = Some(Instant::now());
                    }
                    Some(start) => {
                        if start.elapsed() >= sustained_duration
                            && !MEETING_DETECTED.load(Ordering::SeqCst)
                        {
                            // Check cooldown
                            let should_fire = match last_detection {
                                None => true,
                                Some(last) => last.elapsed() >= cooldown,
                            };

                            if should_fire {
                                MEETING_DETECTED.store(true, Ordering::SeqCst);
                                last_detection = Some(Instant::now());

                                let device_name = MONITORED_DEVICE
                                    .lock()
                                    .map(|n| n.clone())
                                    .unwrap_or_default();

                                let event = MicActivityEvent {
                                    detected: true,
                                    rms_level: rms,
                                    device_name,
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64,
                                };

                                info!(
                                    "🔔 Mic activity detected! RMS={:.4} – emitting event",
                                    rms
                                );

                                if let Err(e) =
                                    app_clone.emit("mic-activity-detected", &event)
                                {
                                    error!(
                                        "Failed to emit mic-activity-detected: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            } else {
                // Mic went quiet – reset
                if activity_start.is_some() {
                    activity_start = None;
                }
                if MEETING_DETECTED.load(Ordering::SeqCst) {
                    MEETING_DETECTED.store(false, Ordering::SeqCst);

                    let device_name = MONITORED_DEVICE
                        .lock()
                        .map(|n| n.clone())
                        .unwrap_or_default();

                    let event = MicActivityEvent {
                        detected: false,
                        rms_level: rms,
                        device_name,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    };

                    let _ = app_clone.emit("mic-activity-stopped", &event);
                }
            }
        }

        info!("Mic activity monitor evaluation task ended");
    });

    info!("✅ Mic activity monitor started successfully");
    Ok(())
}

/// Stop the mic activity monitor and release the audio device.
pub async fn stop_monitoring() -> Result<(), String> {
    info!("Stopping mic activity monitor...");
    IS_MONITORING.store(false, Ordering::SeqCst);
    MEETING_DETECTED.store(false, Ordering::SeqCst);

    // Drop the stream to release the device
    {
        let mut handle = STREAM_HANDLE.lock().unwrap();
        if let Some(stream) = handle.take() {
            drop(stream);
        }
    }

    // Clear leftover samples
    if let Ok(mut buf) = SAMPLE_BUFFER.lock() {
        buf.clear();
    }

    info!("✅ Mic activity monitor stopped");
    Ok(())
}

/// Check if the monitor is currently running.
pub fn is_monitoring() -> bool {
    IS_MONITORING.load(Ordering::SeqCst)
}

/// Check if a meeting has been detected (mic is actively in use).
pub fn is_meeting_detected() -> bool {
    MEETING_DETECTED.load(Ordering::SeqCst)
}

// ============================================================================
// TAURI COMMANDS
// ============================================================================

/// Start mic activity monitoring.  Called from frontend or on app launch.
#[tauri::command]
pub async fn start_mic_activity_monitoring<R: Runtime>(
    app: AppHandle<R>,
) -> Result<(), String> {
    start_monitoring(app).await
}

/// Stop mic activity monitoring.
#[tauri::command]
pub async fn stop_mic_activity_monitoring() -> Result<(), String> {
    stop_monitoring().await
}

/// Get current monitoring status.
#[tauri::command]
pub async fn get_mic_activity_monitoring_status() -> bool {
    is_monitoring()
}

/// Get / set the user preference for mic-activity monitoring.
#[tauri::command]
pub async fn get_mic_activity_monitoring_preference<R: Runtime>(
    app: AppHandle<R>,
) -> Result<bool, String> {
    Ok(load_preference(&app).await)
}

#[tauri::command]
pub async fn set_mic_activity_monitoring_preference<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    save_preference(&app, enabled).await?;

    // Start or stop monitoring based on the new preference
    if enabled {
        if !is_monitoring() {
            start_monitoring(app).await?;
        }
    } else {
        if is_monitoring() {
            stop_monitoring().await?;
        }
    }

    info!("Mic activity monitoring preference set to: {}", enabled);
    Ok(())
}

/// Dismiss the current detection (resets the flag so the user isn't
/// bothered again until the next sustained-activity window).
#[tauri::command]
pub async fn dismiss_mic_activity_detection() -> Result<(), String> {
    MEETING_DETECTED.store(false, Ordering::SeqCst);
    info!("Mic activity detection dismissed by user");
    Ok(())
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        // Ensure monitoring is not running by default
        assert!(!is_monitoring());
        assert!(!is_meeting_detected());
    }

    #[test]
    fn test_meeting_detected_flag() {
        // Set meeting detected
        MEETING_DETECTED.store(true, Ordering::SeqCst);
        assert!(is_meeting_detected());

        // Reset
        MEETING_DETECTED.store(false, Ordering::SeqCst);
        assert!(!is_meeting_detected());
    }

    #[test]
    fn test_monitoring_flag() {
        // Set monitoring
        IS_MONITORING.store(true, Ordering::SeqCst);
        assert!(is_monitoring());

        // Reset
        IS_MONITORING.store(false, Ordering::SeqCst);
        assert!(!is_monitoring());
    }

    #[test]
    fn test_sample_buffer_rms_calculation() {
        // Test RMS calculation with known values
        {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            buf.clear();
            // Add samples: [0.1, 0.1, 0.1, 0.1]
            // RMS = sqrt(4 * 0.01 / 4) = sqrt(0.01) = 0.1
            buf.extend_from_slice(&[0.1_f32, 0.1, 0.1, 0.1]);
        }

        let rms = {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            let sum_sq: f32 = buf.iter().map(|s| s * s).sum();
            let rms = (sum_sq / buf.len() as f32).sqrt();
            buf.clear();
            rms
        };

        assert!((rms - 0.1).abs() < 0.001, "RMS should be ~0.1, got {}", rms);
    }

    #[test]
    fn test_sample_buffer_silence() {
        // Test RMS with silence (all zeros)
        {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            buf.clear();
            buf.extend_from_slice(&[0.0_f32; 100]);
        }

        let rms = {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            let sum_sq: f32 = buf.iter().map(|s| s * s).sum();
            let rms = (sum_sq / buf.len() as f32).sqrt();
            buf.clear();
            rms
        };

        assert!(rms < DEFAULT_RMS_THRESHOLD, "Silence RMS {} should be below threshold {}", rms, DEFAULT_RMS_THRESHOLD);
    }

    #[test]
    fn test_sample_buffer_loud_signal() {
        // Test RMS with a loud signal
        {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            buf.clear();
            buf.extend_from_slice(&[0.5_f32; 100]);
        }

        let rms = {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            let sum_sq: f32 = buf.iter().map(|s| s * s).sum();
            let rms = (sum_sq / buf.len() as f32).sqrt();
            buf.clear();
            rms
        };

        assert!(rms > DEFAULT_RMS_THRESHOLD, "Loud signal RMS {} should be above threshold {}", rms, DEFAULT_RMS_THRESHOLD);
    }

    #[test]
    fn test_sample_buffer_overflow_protection() {
        // Test that the buffer doesn't grow unbounded
        {
            let mut buf = SAMPLE_BUFFER.lock().unwrap();
            buf.clear();
            // Add more than 96_000 samples
            let large_data = vec![0.1_f32; 100_000];
            buf.extend_from_slice(&large_data);
            // Simulate the drain logic from the stream callback
            if buf.len() > 96_000 {
                buf.drain(..buf.len() - 48_000);
            }
            assert!(buf.len() <= 48_000, "Buffer should be bounded to 48000, got {}", buf.len());
            buf.clear();
        }
    }

    #[test]
    fn test_mic_activity_event_serialization() {
        let event = MicActivityEvent {
            detected: true,
            rms_level: 0.05,
            device_name: "Test Microphone".to_string(),
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"detected\":true"));
        assert!(json.contains("\"device_name\":\"Test Microphone\""));

        // Deserialize back
        let deserialized: MicActivityEvent = serde_json::from_str(&json).unwrap();
        assert!(deserialized.detected);
        assert_eq!(deserialized.device_name, "Test Microphone");
        assert!((deserialized.rms_level - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_constants_are_reasonable() {
        // Threshold should be small but positive
        assert!(DEFAULT_RMS_THRESHOLD > 0.0);
        assert!(DEFAULT_RMS_THRESHOLD < 0.1);

        // Sustained duration should be a few seconds
        assert!(DEFAULT_SUSTAINED_SECONDS >= 1);
        assert!(DEFAULT_SUSTAINED_SECONDS <= 30);

        // Cooldown should be reasonable
        assert!(DETECTION_COOLDOWN_SECONDS >= 10);
        assert!(DETECTION_COOLDOWN_SECONDS <= 300);

        // Evaluation interval should be fast enough for responsiveness
        assert!(EVALUATION_INTERVAL_MS >= 50);
        assert!(EVALUATION_INTERVAL_MS <= 1000);
    }

    #[tokio::test]
    async fn test_stop_monitoring_when_not_running() {
        // Stopping when not running should succeed gracefully
        IS_MONITORING.store(false, Ordering::SeqCst);
        let result = stop_monitoring().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dismiss_detection() {
        MEETING_DETECTED.store(true, Ordering::SeqCst);
        assert!(is_meeting_detected());

        let result = dismiss_mic_activity_detection().await;
        assert!(result.is_ok());
        assert!(!is_meeting_detected());
    }
}
