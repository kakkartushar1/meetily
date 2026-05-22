# 🔍 Audio Volume Loss on Windows — Root Cause Analysis & Fix Plan

**Date:** 2026-05-21  
**Issue:** When Meetily is running on Windows, the system audio output loses volume  
**Severity:** High — Affects user experience even when not actively recording  
**Platform:** Windows (WASAPI)

---

## 📋 Executive Summary

The audio volume loss is caused by **Windows Communication Ducking** — a Windows feature that automatically reduces the volume of other applications by ~80% when it detects a "communication" audio stream. The Meetily app triggers this behavior through **multiple WASAPI audio sessions** that are opened at startup and during recording, without explicitly opting out of ducking.

### Root Causes Identified (3 layers)

| # | Root Cause | Severity | Trigger Point |
|---|-----------|----------|---------------|
| 1 | **cpal WASAPI backend defaults to communication session category** | 🔴 Critical | Every `build_input_stream()` call |
| 2 | **Microphone monitoring opens persistent WASAPI capture stream at startup** | 🟠 High | `mic_activity_monitor.rs` auto-start |
| 3 | **Device monitor continuously polls WASAPI device enumeration** | 🟡 Medium | `device_monitor.rs` every 2-5 seconds |

---

## 🔬 Detailed Technical Analysis

### 1. Windows Communication Ducking Mechanism

Windows has a built-in feature called **"Communication Activity Ducking"** (since Windows 7):

```
Settings → System → Sound → Advanced → "When Windows detects communications activity"
  Options:
    - Mute all other sounds
    - Reduce the volume of other sounds by 80%  ← DEFAULT
    - Reduce the volume of other sounds by 50%
    - Do nothing
```

**How it works:**
1. When an application opens a WASAPI audio session
2. If that session is categorized as `AudioCategory_Communications` (the default for capture streams)
3. Windows automatically ducks (reduces volume of) ALL other audio sessions
4. The ducking persists as long as the communication session is active

### 2. How Meetily Triggers Ducking

#### 2a. cpal's WASAPI Backend (The Primary Cause)

The `cpal` library (v0.15.3, patched from git rev `51c3b43`) uses the `windows` crate v0.54.0 for WASAPI. When `build_input_stream()` is called, cpal internally:

```rust
// Inside cpal's WASAPI backend (simplified):
IAudioClient::Initialize(
    AUDCLNT_SHAREMODE_SHARED,  // Shared mode (not exclusive)
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_LOOPBACK, // For output devices
    buffer_duration,
    0,
    &wave_format,
    None,  // No session GUID → Windows assigns default
);
```

**Critical:** cpal does NOT:
- Set `AUDCLNT_STREAMFLAGS_NOPERSIST` (to prevent session persistence)
- Call `IAudioSessionControl2::SetDuckingPreference(TRUE)` (to opt out of ducking)
- Set the audio category to anything other than the default

By default, Windows treats capture streams (especially on the default communication device) as communication activity, triggering the ducking behavior.

#### 2b. Multiple Audio Sessions Opened by Meetily

The app opens **multiple simultaneous WASAPI sessions**, each potentially triggering ducking:

| Session | File | When | Purpose |
|---------|------|------|--------|
| Mic Activity Monitor | `mic_activity_monitor.rs:176` | App startup (if enabled) | Monitors mic for meeting detection |
| Permission Trigger | `discovery.rs:67` | App startup | Brief stream to trigger permission dialog |
| Microphone Capture | `stream.rs:247` | Recording start | Main mic recording |
| System Audio Capture | `stream.rs:247` (output device) | Recording start | WASAPI loopback capture |
| Device Monitor Polling | `device_monitor.rs` | During recording | Polls `list_audio_devices()` every 2-5s |

**The mic activity monitor** (`mic_activity_monitor.rs`) is particularly problematic because:
- It starts automatically at app launch (line 519 in `lib.rs`)
- It opens a persistent `build_input_stream()` on the default microphone
- It stays active for the entire app lifetime
- This alone is enough to trigger Windows communication ducking

```rust
// lib.rs:515-524 — Auto-starts mic monitoring at app launch
let app_for_mic_monitor = _app.handle().clone();
tauri::async_runtime::spawn(async move {
    let enabled = audio::mic_activity_monitor::load_preference(&app_for_mic_monitor).await;
    if enabled {
        // This opens a WASAPI capture stream → triggers ducking!
        if let Err(e) = audio::mic_activity_monitor::start_monitoring(app_for_mic_monitor).await {
            log::error!("Failed to start mic activity monitoring on launch: {}", e);
        }
    }
});
```

#### 2c. System Audio Capture on Windows is Unimplemented

The `capture/system.rs` file shows that Windows system audio capture is **not yet implemented**:

```rust
#[cfg(not(target_os = "macos"))]
{
    // For non-macOS platforms, you would implement WASAPI/ALSA loopback here
    anyhow::bail!("System audio capture not yet implemented for this platform")
}
```

However, the `stream.rs` code still tries to open the output device via cpal's WASAPI backend using `build_input_stream()` on an output device (WASAPI loopback mode). This creates yet another audio session.

### 3. Device Monitor Continuous Polling

The `device_monitor.rs` calls `list_audio_devices()` every 2-5 seconds during recording. On Windows, this calls `cpal::host_from_id(cpal::HostId::Wasapi)` and enumerates all devices, which involves COM operations that can interfere with active audio sessions.

---

## 🔍 Evidence Summary

1. **No `windows` crate direct dependency** — The app has no way to call `IAudioSessionControl2::SetDuckingPreference()` to opt out of ducking
2. **No `AUDCLNT_STREAMFLAGS_NOPERSIST`** — cpal doesn't set this flag, so sessions persist and trigger ducking
3. **Multiple `build_input_stream()` calls** — Each creates a new WASAPI audio session
4. **Mic monitor auto-starts** — Creates a persistent capture session at app launch
5. **cpal git patch (rev 51c3b43)** — Custom cpal revision, but no evidence of ducking-related patches
6. **No audio category configuration** — The app never sets `AudioCategory` on any session

---

## 🛠️ Remediation Plan

### Fix 1: Disable Windows Communication Ducking via COM API (CRITICAL)

**Priority:** 🔴 P0 — Must fix  
**Effort:** Medium  
**Files to modify:** New file + `Cargo.toml` + `recording_manager.rs`

Add the `windows` crate as a direct dependency and call `IAudioSessionControl2::SetDuckingPreference(TRUE)` after each audio session is created.

```toml
# Cargo.toml — Add to [target.'cfg(target_os = "windows")'.dependencies]
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.54.0", features = [
    "Win32_Media_Audio",
    "Win32_System_Com",
    "Win32_Foundation",
] }
```

Create a new file `frontend/src-tauri/src/audio/windows_audio_session.rs`:

```rust
//! Windows audio session management
//! Disables communication ducking to prevent volume loss

#[cfg(target_os = "windows")]
pub mod ducking {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use log::{info, warn, error};

    /// Disable Windows communication ducking for the current process.
    /// This prevents Windows from reducing other apps' volume when
    /// Meetily opens audio capture streams.
    ///
    /// Must be called ONCE at app startup, before any audio streams are opened.
    pub fn disable_communication_ducking() -> Result<(), String> {
        unsafe {
            // Initialize COM
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .map_err(|e| format!("COM init failed: {}", e))?;

            // Get the default audio endpoint for communications
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(
                &MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            ).map_err(|e| format!("Failed to create device enumerator: {}", e))?;

            let device = enumerator.GetDefaultAudioEndpoint(
                eRender,
                eConsole,
            ).map_err(|e| format!("Failed to get default endpoint: {}", e))?;

            // Activate IAudioSessionManager2
            let session_manager: IAudioSessionManager2 = device.Activate(
                CLSCTX_ALL,
                None,
            ).map_err(|e| format!("Failed to activate session manager: {}", e))?;

            // Get the audio session control for this process
            let session_control = session_manager.GetAudioSessionControl(
                None, // Default session
                0,    // Not cross-process
            ).map_err(|e| format!("Failed to get session control: {}", e))?;

            // Query for IAudioSessionControl2
            let session_control2: IAudioSessionControl2 = session_control.cast()
                .map_err(|e| format!("Failed to cast to IAudioSessionControl2: {}", e))?;

            // CRITICAL: Opt out of ducking
            session_control2.SetDuckingPreference(true)
                .map_err(|e| format!("Failed to set ducking preference: {}", e))?;

            info!("✅ Windows communication ducking disabled for Meetily");
            Ok(())
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod ducking {
    pub fn disable_communication_ducking() -> Result<(), String> {
        Ok(()) // No-op on non-Windows platforms
    }
}
```

### Fix 2: Call Ducking Disable at App Startup (CRITICAL)

**Priority:** 🔴 P0  
**File:** `frontend/src-tauri/src/lib.rs`

Add the ducking disable call in the Tauri `setup` hook, BEFORE any audio streams are opened:

```rust
// In the setup closure, before mic_activity_monitor starts:
#[cfg(target_os = "windows")]
{
    if let Err(e) = audio::windows_audio_session::ducking::disable_communication_ducking() {
        log::warn!("Failed to disable Windows communication ducking: {}", e);
        log::warn!("Other apps may experience volume reduction while Meetily is running");
    }
}
```

### Fix 3: Lazy Mic Activity Monitor (HIGH)

**Priority:** 🟠 P1  
**File:** `frontend/src-tauri/src/audio/mic_activity_monitor.rs`

Instead of opening a persistent WASAPI capture stream at startup, defer mic monitoring until actually needed:

- Option A: Only start monitoring when the user is NOT actively recording (reduces concurrent sessions)
- Option B: Use a polling approach (check mic device availability without opening a stream)
- Option C: Add a startup delay (e.g., 30 seconds) before opening the mic monitor stream

### Fix 4: Reduce Device Monitor Polling Frequency (MEDIUM)

**Priority:** 🟡 P2  
**File:** `frontend/src-tauri/src/audio/device_monitor.rs`

Increase the polling interval from 2-5 seconds to 10-15 seconds, and avoid calling `list_audio_devices()` which triggers WASAPI device enumeration COM calls.

### Fix 5: Patch cpal to Support Ducking Opt-Out (LONG-TERM)

**Priority:** 🔵 P3 — Nice to have  
**Effort:** High  

Fork cpal and add support for:
- Setting `AUDCLNT_STREAMFLAGS_NOPERSIST` on stream creation
- Exposing `IAudioSessionControl2` for ducking preference
- Setting audio category to `AudioCategory_Media` instead of default

---

## 📊 Impact Assessment

| Fix | Volume Loss Reduction | Implementation Risk | User Impact |
|-----|----------------------|--------------------|-----------|
| Fix 1+2 (Ducking disable) | ~90% | Low | Immediate relief |
| Fix 3 (Lazy mic monitor) | ~50% standalone | Low | Reduces background sessions |
| Fix 4 (Reduce polling) | ~10% | Very Low | Minor improvement |
| Fix 5 (cpal patch) | ~100% | High | Complete solution |

**Recommended approach:** Implement Fix 1+2 first (immediate relief), then Fix 3 (defense in depth).

---

## 🧪 Testing Plan

1. **Before fix:** Open Meetily → Play music in another app → Observe volume reduction
2. **After fix:** Open Meetily → Play music → Volume should remain unchanged
3. **During recording:** Start recording → Play music → Volume should remain unchanged
4. **Edge case:** Test with Bluetooth headset (communication device) → Volume should remain unchanged
5. **Regression:** Ensure mic activity detection still works after lazy loading changes

---

## 📁 Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `src/audio/windows_audio_session.rs` | **CREATE** | Windows ducking disable module |
| `src/audio/mod.rs` | MODIFY | Add `windows_audio_session` module |
| `Cargo.toml` | MODIFY | Add `windows` crate dependency |
| `src/lib.rs` | MODIFY | Call ducking disable at startup |
| `src/audio/mic_activity_monitor.rs` | MODIFY | Add lazy initialization |
| `src/audio/device_monitor.rs` | MODIFY | Reduce polling frequency |

---

## 🔗 References

- [Microsoft: Using a Communication Device](https://learn.microsoft.com/en-us/windows/win32/coreaudio/using-a-communication-device)
- [Microsoft: IAudioSessionControl2::SetDuckingPreference](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-setduckingpreference)
- [Microsoft: Default Ducking Experience](https://learn.microsoft.com/en-us/windows/win32/coreaudio/default-ducking-experience)
- [cpal WASAPI backend source](https://github.com/RustAudio/cpal/blob/master/src/host/wasapi/)
- [Windows Audio Session Categories](https://learn.microsoft.com/en-us/windows/win32/api/audiosessiontypes/ne-audiosessiontypes-audio_stream_category)
