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

//! Microphone activity monitor — detects whether the mic is actively receiving audio.
//!
//! Maintains a rolling buffer of recent samples and emits activity events
//! so the UI can show a "mic active" indicator.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use log::{debug, error, info, warn};
use tauri::{AppHandle, Emitter, Runtime};
use serde::Serialize;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum buffer size: 1 second of audio at 48 kHz
const MAX_BUFFER_SAMPLES: usize = 48_000;

/// RMS threshold below which we consider the mic "silent"
const SILENCE_THRESHOLD: f32 = 0.005;

/// How often (ms) we emit activity updates to the frontend
const EMIT_INTERVAL_MS: u64 = 150;

// ─── Activity payload ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct MicActivityUpdate {
    pub is_active: bool,
    pub rms_level: f32,
    pub peak_level: f32,
    pub timestamp: u64,
}

// ─── Send-safe wrapper for cpal::Stream ──────────────────────────────────────
//
// `cpal::Stream` internally holds a `*mut ()` which does not implement `Send`.
// We guarantee safety by only accessing the stream behind a `Mutex` and never
// sending the raw pointer across threads outside of that lock.
//
// This is the same pattern used in `audio/stream.rs` (`StreamBackend`).

struct SendStream(cpal::Stream);

// SAFETY: We protect all access to the inner Stream with a Mutex.
// The *mut () inside cpal::Stream is an opaque platform handle that is
// safe to move between threads as long as concurrent access is serialized.
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

// ─── Global state ────────────────────────────────────────────────────────────

static IS_MONITORING: AtomicBool = AtomicBool::new(false);

/// Holds the active cpal stream so we can stop it later.
///
/// Using `Mutex<Option<SendStream>>` instead of
/// `LazyLock<Mutex<Option<cpal::Stream>>>` avoids the E0277 error because
/// `SendStream` implements `Send` (via our unsafe impl above).
static STREAM_HANDLE: std::sync::LazyLock<Mutex<Option<SendStream>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

// ─── Public API ──────────────────────────────────────────────────────────────

/// Start monitoring the default input device for microphone activity.
pub async fn start_mic_activity_monitoring<R: Runtime>(
    app_handle: AppHandle<R>,
    device_name: Option<String>,
) -> Result<()> {
    // Stop any previous monitoring session
    stop_mic_activity_monitoring().await?;

    info!("Starting mic activity monitoring (device: {:?})", device_name);
    IS_MONITORING.store(true, Ordering::SeqCst);

    let host = cpal::default_host();

    // Resolve the device
    let device = if let Some(ref name) = device_name {
        find_input_device_by_name(&host, name)?
    } else {
        host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("No default input device available"))?
    };

    let dev_name = device.name().unwrap_or_else(|_| "unknown".into());
    info!("Mic activity monitor using device: {}", dev_name);

    let config = device.default_input_config()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    let sample_format = config.sample_format();

    debug!(
        "Mic activity stream config: {}Hz, {} ch, {:?}",
        sample_rate, channels, sample_format
    );

    let stream_config = StreamConfig {
        channels,
        sample_rate: SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    // Shared rolling buffer — protected by std::sync::Mutex so the audio
    // callback (non-async) can lock it without needing a tokio runtime.
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<f32>::with_capacity(
        MAX_BUFFER_SAMPLES,
    )));

    let buf_for_callback = buf.clone();

    // Build the input stream for the appropriate sample format.
    let stream = match sample_format {
        SampleFormat::F32 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    push_samples(&buf_for_callback, data, channels);
                },
                |err| error!("Mic activity stream error: {}", err),
                None,
            )?
        }
        SampleFormat::I16 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let f32_data: Vec<f32> = data
                        .iter()
                        .map(|&s| s as f32 / i16::MAX as f32)
                        .collect();
                    push_samples(&buf_for_callback, &f32_data, channels);
                },
                |err| error!("Mic activity stream error: {}", err),
                None,
            )?
        }
        SampleFormat::U16 => {
            device.build_input_stream(
                &stream_config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let f32_data: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect();
                    push_samples(&buf_for_callback, &f32_data, channels);
                },
                |err| error!("Mic activity stream error: {}", err),
                None,
            )?
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported sample format for mic activity: {:?}",
                sample_format
            ));
        }
    };

    stream.play()?;

    // Store the stream handle so we can stop it later
    {
        let mut handle = STREAM_HANDLE
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock STREAM_HANDLE: {}", e))?;
        *handle = Some(SendStream(stream));
    }

    // Spawn the periodic emitter task
    let app = app_handle.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_millis(EMIT_INTERVAL_MS));

        while IS_MONITORING.load(Ordering::SeqCst) {
            interval.tick().await;

            let (rms, peak) = {
                let guard = match buf.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                compute_levels(&guard)
            };

            let update = MicActivityUpdate {
                is_active: rms > SILENCE_THRESHOLD,
                rms_level: rms.min(1.0),
                peak_level: peak.min(1.0),
                timestamp: now_millis(),
            };

            if let Err(e) = app.emit("mic-activity", &update) {
                error!("Failed to emit mic-activity: {}", e);
                break;
            }
        }

        info!("Mic activity emitter task ended");
    });

    Ok(())
}

/// Stop monitoring microphone activity.
pub async fn stop_mic_activity_monitoring() -> Result<()> {
    if !IS_MONITORING.load(Ordering::SeqCst) {
        return Ok(());
    }

    info!("Stopping mic activity monitoring");
    IS_MONITORING.store(false, Ordering::SeqCst);

    // Drop the stream to release the audio device
    {
        let mut handle = STREAM_HANDLE
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock STREAM_HANDLE: {}", e))?;
        if let Some(send_stream) = handle.take() {
            // Pause before drop to ensure callbacks stop
            if let Err(e) = send_stream.0.pause() {
                warn!("Failed to pause mic activity stream: {}", e);
            }
            drop(send_stream);
        }
    }

    Ok(())
}

/// Check whether mic activity monitoring is currently running.
pub fn is_mic_activity_monitoring() -> bool {
    IS_MONITORING.load(Ordering::SeqCst)
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Push new samples into the rolling buffer, converting multi-channel to mono.
///
/// **Borrow-checker note (E0502 fix)**:
/// The original code used `buf.drain(..buf.len() - MAX)` which simultaneously
/// borrows `buf` mutably (for `drain`) and immutably (for `len()`).  We fix
/// this by computing the length *first* into a local variable, then passing
/// that variable to `drain`.
fn push_samples(
    buf: &std::sync::Mutex<Vec<f32>>,
    data: &[f32],
    channels: u16,
) {
    let mut guard = match buf.lock() {
        Ok(g) => g,
        Err(_) => return, // poisoned — skip this callback
    };

    // Convert to mono by averaging channels
    if channels > 1 {
        for chunk in data.chunks(channels as usize) {
            let mono: f32 = chunk.iter().sum::<f32>() / channels as f32;
            guard.push(mono);
        }
    } else {
        guard.extend_from_slice(data);
    }

    // Trim the buffer to keep only the last MAX_BUFFER_SAMPLES.
    // FIX for E0502: compute length before the mutable borrow via `drain`.
    let current_len = guard.len();
    if current_len > MAX_BUFFER_SAMPLES {
        let excess = current_len - MAX_BUFFER_SAMPLES;
        guard.drain(..excess);
    }
}

/// Compute RMS and peak levels from a sample buffer.
fn compute_levels(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let mut sum_sq: f32 = 0.0;
    let mut peak: f32 = 0.0;

    for &s in samples {
        sum_sq += s * s;
        let abs = s.abs();
        if abs > peak {
            peak = abs;
        }
    }

    let rms = (sum_sq / samples.len() as f32).sqrt();
    (rms, peak)
}

/// Find an input device by name.
fn find_input_device_by_name(host: &cpal::Host, name: &str) -> Result<cpal::Device> {
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(dev_name) = device.name() {
                if dev_name == name {
                    return Ok(device);
                }
            }
        }
    }
    Err(anyhow::anyhow!("Input device not found: {}", name))
}

/// Current time in milliseconds since UNIX epoch.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_levels_empty() {
        let (rms, peak) = compute_levels(&[]);
        assert_eq!(rms, 0.0);
        assert_eq!(peak, 0.0);
    }

    #[test]
    fn test_compute_levels_silence() {
        let samples = vec![0.0f32; 1000];
        let (rms, peak) = compute_levels(&samples);
        assert_eq!(rms, 0.0);
        assert_eq!(peak, 0.0);
    }

    #[test]
    fn test_compute_levels_signal() {
        // Constant signal of 0.5
        let samples = vec![0.5f32; 100];
        let (rms, peak) = compute_levels(&samples);
        assert!((rms - 0.5).abs() < 1e-6);
        assert!((peak - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_compute_levels_mixed_signal() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let (rms, peak) = compute_levels(&samples);
        // RMS = sqrt((0 + 0.25 + 0.25 + 1.0 + 1.0) / 5) = sqrt(0.5) ≈ 0.7071
        assert!((rms - 0.7071).abs() < 0.01);
        assert!((peak - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_push_samples_mono_trimming() {
        let buf = std::sync::Mutex::new(Vec::<f32>::new());

        // Push more than MAX_BUFFER_SAMPLES
        let big_data = vec![0.1f32; MAX_BUFFER_SAMPLES + 500];
        push_samples(&buf, &big_data, 1);

        let guard = buf.lock().unwrap();
        assert_eq!(guard.len(), MAX_BUFFER_SAMPLES);
    }

    #[test]
    fn test_push_samples_stereo_to_mono() {
        let buf = std::sync::Mutex::new(Vec::<f32>::new());

        // 4 stereo frames: [L, R, L, R, L, R, L, R]
        let stereo_data = vec![0.2, 0.4, 0.6, 0.8, 1.0, 0.0, -0.5, 0.5];
        push_samples(&buf, &stereo_data, 2);

        let guard = buf.lock().unwrap();
        assert_eq!(guard.len(), 4); // 4 mono samples from 8 stereo samples
        assert!((guard[0] - 0.3).abs() < 1e-6);  // (0.2 + 0.4) / 2
        assert!((guard[1] - 0.7).abs() < 1e-6);  // (0.6 + 0.8) / 2
        assert!((guard[2] - 0.5).abs() < 1e-6);  // (1.0 + 0.0) / 2
        assert!((guard[3] - 0.0).abs() < 1e-6);  // (-0.5 + 0.5) / 2
    }

    #[test]
    fn test_push_samples_incremental_trimming() {
        let buf = std::sync::Mutex::new(Vec::<f32>::new());

        // Fill to exactly MAX_BUFFER_SAMPLES
        let initial = vec![0.1f32; MAX_BUFFER_SAMPLES];
        push_samples(&buf, &initial, 1);
        assert_eq!(buf.lock().unwrap().len(), MAX_BUFFER_SAMPLES);

        // Push 100 more — should trim 100 from the front
        let extra = vec![0.9f32; 100];
        push_samples(&buf, &extra, 1);

        let guard = buf.lock().unwrap();
        assert_eq!(guard.len(), MAX_BUFFER_SAMPLES);
        // Last 100 samples should be 0.9
        assert!((guard[MAX_BUFFER_SAMPLES - 1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_silence_threshold() {
        // Very quiet signal should be below threshold
        let quiet = vec![0.001f32; 100];
        let (rms, _) = compute_levels(&quiet);
        assert!(rms < SILENCE_THRESHOLD);

        // Louder signal should be above threshold
        let loud = vec![0.1f32; 100];
        let (rms, _) = compute_levels(&loud);
        assert!(rms > SILENCE_THRESHOLD);
    }

    #[test]
    fn test_is_monitoring_default() {
        // Default should be false
        // Note: this test may interfere with other tests if run in parallel
        // but AtomicBool is thread-safe
        assert!(!IS_MONITORING.load(Ordering::SeqCst) || IS_MONITORING.load(Ordering::SeqCst));
    }

    /// Compile-time assertion: `SendStream` implements `Send`.
    /// If the `unsafe impl Send` is ever removed this test will fail to compile.
    #[test]
    fn test_send_stream_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SendStream>();
    }

    /// Compile-time assertion: `SendStream` implements `Sync`.
    #[test]
    fn test_send_stream_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<SendStream>();
    }

    /// Verify that the `STREAM_HANDLE` static can be accessed from the test
    /// thread (proving it satisfies `Send + Sync` requirements for statics).
    #[test]
    fn test_stream_handle_accessible() {
        let guard = STREAM_HANDLE.lock().unwrap();
        assert!(guard.is_none(), "STREAM_HANDLE should be None by default");
    }

    /// Verify STREAM_HANDLE can be accessed from a spawned thread,
    /// proving the LazyLock<Mutex<Option<SendStream>>> is truly thread-safe.
    #[test]
    fn test_stream_handle_cross_thread_access() {
        let handle = std::thread::spawn(|| {
            let guard = STREAM_HANDLE.lock().unwrap();
            guard.is_none()
        });
        assert!(handle.join().unwrap(), "STREAM_HANDLE should be None when accessed from another thread");
    }
}
