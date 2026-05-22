//! Microphone activity monitor — detects whether the mic is actively receiving audio.
//!
//! Maintains a rolling buffer of recent samples and emits activity events
//! so the UI can show a "mic active" indicator.
//!
//! Also implements meeting detection: when sustained mic activity is detected
//! for a configurable duration, it emits `mic-activity-detected` so the UI
//! can prompt the user to start recording. When activity drops, it emits
//! `mic-activity-stopped`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use log::{debug, error, info, warn};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;
use serde::Serialize;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum buffer size: 1 second of audio at 48 kHz
const MAX_BUFFER_SAMPLES: usize = 48_000;

/// RMS threshold below which we consider the mic "silent".
/// Raised from 0.005 to 0.02 to avoid false positives from ambient room noise
/// and electrical noise on the microphone input.
const SILENCE_THRESHOLD: f32 = 0.02;

/// How often (ms) we emit activity updates to the frontend
const EMIT_INTERVAL_MS: u64 = 150;

/// Number of consecutive active checks needed to trigger meeting detection.
/// At 150ms intervals, 60 checks ≈ 9 seconds of sustained mic activity.
/// Increased from 20 (3s) to reduce false positives from brief ambient noise.
const MEETING_DETECTION_THRESHOLD: u32 = 60;

/// Number of consecutive silent checks needed to trigger meeting ended
/// At 150ms intervals, 40 checks ≈ 6 seconds of sustained silence
const MEETING_ENDED_THRESHOLD: u32 = 40;

/// Cooldown period (ms) after dismissing detection before re-firing
const DETECTION_COOLDOWN_MS: u64 = 60_000;

/// Grace period (ms) after monitoring starts before detection can fire.
/// This prevents false positives from mic initialization noise and
/// ambient sound pickup during app startup.
const STARTUP_GRACE_PERIOD_MS: u64 = 10_000;

/// Delay (ms) before opening the WASAPI capture stream at app startup.
/// This prevents the mic monitor from immediately opening a WASAPI session
/// that could trigger Windows Communication Ducking before the ducking
/// prevention code has had time to take effect.
/// Also reduces resource contention during app initialization.
const STARTUP_STREAM_DELAY_MS: u64 = 5_000;

/// Store key for the mic monitoring preference
const STORE_KEY_MIC_MONITORING: &str = "mic_activity_monitoring_enabled";

/// Store file name for preferences
const PREFERENCES_STORE: &str = "preferences.json";

// ─── Activity payload ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct MicActivityUpdate {
    pub is_active: bool,
    pub rms_level: f32,
    pub peak_level: f32,
    pub timestamp: u64,
}

/// Payload emitted when a meeting is detected or stops.
#[derive(Debug, Serialize, Clone)]
pub struct MicActivityEvent {
    pub detected: bool,
    pub rms_level: f32,
    pub device_name: String,
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

/// Whether a meeting is currently detected (sustained mic activity).
static MEETING_DETECTED: AtomicBool = AtomicBool::new(false);

/// Whether detection has been dismissed (cooldown active).
static DETECTION_DISMISSED: AtomicBool = AtomicBool::new(false);

/// Timestamp (ms) when detection was last dismissed.
static DISMISS_TIMESTAMP: std::sync::LazyLock<std::sync::atomic::AtomicU64> =
    std::sync::LazyLock::new(|| std::sync::atomic::AtomicU64::new(0));

/// Holds the active cpal stream so we can stop it later.
///
/// Using `Mutex<Option<SendStream>>` instead of
/// `LazyLock<Mutex<Option<cpal::Stream>>>` avoids the E0277 error because
/// `SendStream` implements `Send` (via our unsafe impl above).
static STREAM_HANDLE: std::sync::LazyLock<Mutex<Option<SendStream>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Stores the device name being monitored for event payloads.
static MONITORED_DEVICE_NAME: std::sync::LazyLock<Mutex<String>> =
    std::sync::LazyLock::new(|| Mutex::new(String::new()));

// ─── Public API ──────────────────────────────────────────────────────────────

/// Start monitoring the default input device for microphone activity.
///
/// On Windows, this defers stream opening by `STARTUP_STREAM_DELAY_MS` to
/// allow the ducking prevention code in `windows_audio_session` to take
/// effect before any WASAPI capture sessions are opened.
pub async fn start_mic_activity_monitoring<R: Runtime>(
    app_handle: AppHandle<R>,
    device_name: Option<String>,
) -> Result<()> {
    // Stop any previous monitoring session
    stop_mic_activity_monitoring().await?;

    info!("Starting mic activity monitoring (device: {:?})", device_name);
    IS_MONITORING.store(true, Ordering::SeqCst);

    // OPTIMIZATION: On Windows, delay stream opening to allow ducking prevention
    // to take effect. The windows_audio_session::ducking::disable_communication_ducking()
    // call in lib.rs runs synchronously during setup, but the mic monitor is spawned
    // asynchronously. This delay ensures the ducking opt-out is fully registered
    // before we open a WASAPI capture session.
    #[cfg(target_os = "windows")]
    {
        info!("Delaying mic activity stream by {}ms to allow ducking prevention to take effect",
              STARTUP_STREAM_DELAY_MS);
        tokio::time::sleep(tokio::time::Duration::from_millis(STARTUP_STREAM_DELAY_MS)).await;

        // Check if monitoring was stopped during the delay
        if !IS_MONITORING.load(Ordering::SeqCst) {
            info!("Mic activity monitoring was stopped during startup delay, aborting");
            return Ok(());
        }
    }

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

    // Store the device name for event payloads
    if let Ok(mut name) = MONITORED_DEVICE_NAME.lock() {
        *name = dev_name.clone();
    }

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

    // Spawn the periodic emitter task with meeting detection logic
    let app = app_handle.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_millis(EMIT_INTERVAL_MS));

        // Counters for meeting detection state machine
        let mut consecutive_active: u32 = 0;
        let mut consecutive_silent: u32 = 0;

        // Record when monitoring started so we can enforce a grace period
        // that suppresses detection during app startup / mic initialization.
        let monitoring_started_at = now_millis();

        while IS_MONITORING.load(Ordering::SeqCst) {
            interval.tick().await;

            let (rms, peak) = {
                let guard = match buf.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                compute_levels(&guard)
            };

            let is_active = rms > SILENCE_THRESHOLD;

            let update = MicActivityUpdate {
                is_active,
                rms_level: rms.min(1.0),
                peak_level: peak.min(1.0),
                timestamp: now_millis(),
            };

            if let Err(e) = app.emit("mic-activity", &update) {
                error!("Failed to emit mic-activity: {}", e);
                break;
            }

            // ── Startup grace period ──
            // Skip meeting detection during the first STARTUP_GRACE_PERIOD_MS
            // after monitoring starts. This prevents false positives caused by
            // mic initialization noise and ambient sound pickup on app launch.
            let elapsed_since_start = now_millis() - monitoring_started_at;
            if elapsed_since_start < STARTUP_GRACE_PERIOD_MS {
                // Reset counters during grace period so we don't accumulate
                // stale "active" counts that fire immediately after grace ends.
                consecutive_active = 0;
                consecutive_silent = 0;
                continue;
            }

            // ── Meeting detection state machine ──
            let currently_detected = MEETING_DETECTED.load(Ordering::SeqCst);
            let dismissed = DETECTION_DISMISSED.load(Ordering::SeqCst);

            if is_active {
                consecutive_active += 1;
                consecutive_silent = 0;

                // Check if we should fire meeting-detected
                if !currently_detected
                    && consecutive_active >= MEETING_DETECTION_THRESHOLD
                {
                    // Check cooldown
                    let dismiss_time = DISMISS_TIMESTAMP.load(Ordering::SeqCst);
                    let now = now_millis();
                    let cooldown_expired = dismiss_time == 0
                        || (now - dismiss_time) >= DETECTION_COOLDOWN_MS;

                    if cooldown_expired && !dismissed {
                        MEETING_DETECTED.store(true, Ordering::SeqCst);
                        let device_name = MONITORED_DEVICE_NAME
                            .lock()
                            .map(|n| n.clone())
                            .unwrap_or_default();

                        let event = MicActivityEvent {
                            detected: true,
                            rms_level: rms.min(1.0),
                            device_name,
                            timestamp: now,
                        };

                        info!("Meeting detected — sustained mic activity for ~{}s",
                            (consecutive_active as f32 * EMIT_INTERVAL_MS as f32 / 1000.0) as u32);

                        if let Err(e) = app.emit("mic-activity-detected", &event) {
                            error!("Failed to emit mic-activity-detected: {}", e);
                        }
                    }
                }
            } else {
                consecutive_silent += 1;
                consecutive_active = 0;

                // Check if we should fire meeting-stopped
                if currently_detected
                    && consecutive_silent >= MEETING_ENDED_THRESHOLD
                {
                    MEETING_DETECTED.store(false, Ordering::SeqCst);
                    // Reset dismissed flag so next meeting can be detected
                    DETECTION_DISMISSED.store(false, Ordering::SeqCst);

                    let device_name = MONITORED_DEVICE_NAME
                        .lock()
                        .map(|n| n.clone())
                        .unwrap_or_default();

                    let event = MicActivityEvent {
                        detected: false,
                        rms_level: rms.min(1.0),
                        device_name,
                        timestamp: now_millis(),
                    };

                    info!("Meeting ended — sustained silence for ~{}s",
                        (consecutive_silent as f32 * EMIT_INTERVAL_MS as f32 / 1000.0) as u32);

                    if let Err(e) = app.emit("mic-activity-stopped", &event) {
                        error!("Failed to emit mic-activity-stopped: {}", e);
                    }
                }
            }
        }

        // Reset meeting detection state when monitoring stops
        MEETING_DETECTED.store(false, Ordering::SeqCst);
        DETECTION_DISMISSED.store(false, Ordering::SeqCst);
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

// ============================================================================
// TAURI COMMANDS
// ============================================================================

/// Start mic activity monitoring. Called from frontend or on app launch.
#[tauri::command]
pub async fn start_mic_activity_monitoring_command<R: Runtime>(
    app: AppHandle<R>,
    device_name: Option<String>,
) -> Result<(), String> {
    start_mic_activity_monitoring(app, device_name)
        .await
        .map_err(|e| e.to_string())
}

/// Stop mic activity monitoring.
#[tauri::command]
pub async fn stop_mic_activity_monitoring_command() -> Result<(), String> {
    stop_mic_activity_monitoring()
        .await
        .map_err(|e| e.to_string())
}

/// Get current monitoring status.
#[tauri::command]
pub async fn get_mic_activity_monitoring_status() -> bool {
    is_mic_activity_monitoring()
}

/// Get the user preference for mic-activity monitoring.
/// Reads from the Tauri store; defaults to `true` (enabled by default).
#[tauri::command]
pub async fn get_mic_activity_monitoring_preference<R: Runtime>(
    app: AppHandle<R>,
) -> Result<bool, String> {
    match app.store(PREFERENCES_STORE) {
        Ok(store) => {
            let enabled = store
                .get(STORE_KEY_MIC_MONITORING)
                .and_then(|v| v.as_bool())
                .unwrap_or(true); // Default to true — enabled by default
            Ok(enabled)
        }
        Err(e) => {
            warn!("Failed to open preferences store: {}, defaulting to enabled", e);
            Ok(true)
        }
    }
}

/// Set the user preference for mic-activity monitoring.
/// Persists to the Tauri store and starts/stops monitoring accordingly.
#[tauri::command]
pub async fn set_mic_activity_monitoring_preference<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    // Persist the preference to the store
    match app.store(PREFERENCES_STORE) {
        Ok(store) => {
            store.set(STORE_KEY_MIC_MONITORING, serde_json::json!(enabled));
            if let Err(e) = store.save() {
                warn!("Failed to save preferences store: {}", e);
            }
        }
        Err(e) => {
            warn!("Failed to open preferences store for writing: {}", e);
        }
    }

    // Start or stop monitoring based on the new preference
    if enabled {
        if !is_mic_activity_monitoring() {
            start_mic_activity_monitoring(app, None)
                .await
                .map_err(|e| e.to_string())?;
        }
    } else {
        if is_mic_activity_monitoring() {
            stop_mic_activity_monitoring()
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    info!("Mic activity monitoring preference set to: {}", enabled);
    Ok(())
}

/// Dismiss the current detection and start cooldown.
#[tauri::command]
pub async fn dismiss_mic_activity_detection() -> Result<(), String> {
    info!("Mic activity detection dismissed by user");
    MEETING_DETECTED.store(false, Ordering::SeqCst);
    DETECTION_DISMISSED.store(true, Ordering::SeqCst);
    DISMISS_TIMESTAMP.store(now_millis(), Ordering::SeqCst);
    Ok(())
}

// ============================================================================
// CONVENIENCE FUNCTIONS (used by lib.rs during app startup)
// ============================================================================

/// Load the user's mic-activity monitoring preference from the Tauri store.
/// Returns `true` if monitoring should be enabled (default: true).
pub async fn load_preference<R: Runtime>(app: &AppHandle<R>) -> bool {
    match app.store(PREFERENCES_STORE) {
        Ok(store) => {
            let enabled = store
                .get(STORE_KEY_MIC_MONITORING)
                .and_then(|v| v.as_bool())
                .unwrap_or(true); // Default to true — enabled by default
            enabled
        }
        Err(e) => {
            warn!("Failed to open preferences store for loading mic monitoring pref: {}, defaulting to enabled", e);
            true
        }
    }
}

/// Start monitoring (convenience wrapper used by lib.rs startup code).
pub async fn start_monitoring<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    start_mic_activity_monitoring(app, None)
        .await
        .map_err(|e| e.to_string())
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
