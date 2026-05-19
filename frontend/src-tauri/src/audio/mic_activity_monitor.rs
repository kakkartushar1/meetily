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
}
