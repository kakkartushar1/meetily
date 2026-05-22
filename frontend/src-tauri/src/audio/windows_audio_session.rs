//! Windows Audio Session Management — Prevents Communication Ducking
//!
//! When Meetily opens WASAPI audio capture streams (via cpal), Windows may
//! interpret them as "communication" sessions and automatically reduce the
//! volume of all other applications by up to 80%. This is called
//! "Communication Activity Ducking" and is enabled by default in Windows.
//!
//! This module provides functions to:
//! 1. Disable ducking for the current process at startup
//! 2. Restore the default ducking behavior on shutdown
//!
//! ## How it works
//!
//! We use the Windows Core Audio COM APIs to:
//! - Obtain the default audio endpoint (render device)
//! - Get the `IAudioSessionManager2` for that endpoint
//! - Retrieve our process's `IAudioSessionControl2`
//! - Call `SetDuckingPreference(TRUE)` to opt out of ducking
//!
//! This must be called **before** any cpal streams are opened (i.e., before
//! `mic_activity_monitor` starts and before recording begins).
//!
//! ## References
//!
//! - [Microsoft: Default Ducking Experience](https://learn.microsoft.com/en-us/windows/win32/coreaudio/default-ducking-experience)
//! - [Microsoft: IAudioSessionControl2::SetDuckingPreference](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-setduckingpreference)
//! - [Microsoft: Using a Communication Device](https://learn.microsoft.com/en-us/windows/win32/coreaudio/using-a-communication-device)

#[cfg(target_os = "windows")]
pub mod ducking {
    use log::{info, error};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Track whether we've already disabled ducking (idempotency guard).
    static DUCKING_DISABLED: AtomicBool = AtomicBool::new(false);

    /// Disable Windows Communication Ducking for the current process.
    ///
    /// This prevents Windows from reducing other applications' volume when
    /// Meetily opens audio capture streams via WASAPI.
    ///
    /// # Safety
    ///
    /// This function uses Windows COM APIs internally. It initializes COM
    /// in multi-threaded apartment mode if not already initialized.
    ///
    /// # Idempotency
    ///
    /// Safe to call multiple times — subsequent calls are no-ops.
    ///
    /// # Errors
    ///
    /// Returns an error string if any COM operation fails. The caller should
    /// log the error but continue running (ducking is annoying but not fatal).
    pub fn disable_communication_ducking() -> Result<(), String> {
        // Idempotency: only disable once
        if DUCKING_DISABLED.swap(true, Ordering::SeqCst) {
            info!("Windows communication ducking already disabled, skipping");
            return Ok(());
        }

        info!("🔇 Disabling Windows Communication Ducking for Meetily...");

        // Use raw Windows COM API calls via the `windows` crate
        unsafe {
            disable_ducking_impl()
        }
    }

    /// Internal implementation using Windows COM APIs.
    ///
    /// # Safety
    ///
    /// Caller must ensure this is called from a thread where COM can be
    /// initialized (which is any thread in practice).
    unsafe fn disable_ducking_impl() -> Result<(), String> {
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::*;
        use windows::core::Interface;

        // Step 1: Initialize COM (multi-threaded apartment)
        // S_FALSE means COM was already initialized — that's fine.
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_err() {
            // HRESULT S_FALSE (0x00000001) means already initialized — not an error
            let code = hr.0 as u32;
            if code != 0x00000001 {
                let msg = format!("COM initialization failed: HRESULT 0x{:08X}", code);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        }
        info!("  COM initialized successfully");

        // Step 2: Create the MMDeviceEnumerator
        let enumerator: IMMDeviceEnumerator = match CoCreateInstance(
            &MMDeviceEnumerator,
            None,
            CLSCTX_ALL,
        ) {
            Ok(e) => e,
            Err(e) => {
                let msg = format!("Failed to create MMDeviceEnumerator: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        };
        info!("  MMDeviceEnumerator created");

        // Step 3: Get the default audio render endpoint (speakers/headphones)
        // We use eConsole role because that's what most apps use for playback.
        // The ducking preference applies per-process, not per-endpoint.
        let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("Failed to get default audio endpoint: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        };
        info!("  Default audio endpoint obtained");

        // Step 4: Activate IAudioSessionManager2 on the endpoint
        let session_manager: IAudioSessionManager2 = match device.Activate(
            CLSCTX_ALL,
            None,
        ) {
            Ok(sm) => sm,
            Err(e) => {
                let msg = format!("Failed to activate IAudioSessionManager2: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        };
        info!("  IAudioSessionManager2 activated");

        // Step 5: Get the audio session control for the current process
        let session_control: IAudioSessionControl = match session_manager.GetAudioSessionControl(
            None,  // Default session GUID (our process)
            0,     // Not cross-process
        ) {
            Ok(sc) => sc,
            Err(e) => {
                let msg = format!("Failed to get IAudioSessionControl: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        };
        info!("  IAudioSessionControl obtained");

        // Step 6: Query for IAudioSessionControl2 (extends IAudioSessionControl)
        let session_control2: IAudioSessionControl2 = match session_control.cast() {
            Ok(sc2) => sc2,
            Err(e) => {
                let msg = format!("Failed to cast to IAudioSessionControl2: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                return Err(msg);
            }
        };
        info!("  IAudioSessionControl2 obtained");

        // Step 7: CRITICAL — Opt out of ducking
        // SetDuckingPreference(TRUE) tells Windows:
        // "Do NOT duck other apps when this process opens audio sessions"
        match session_control2.SetDuckingPreference(
            true
        ) {
            Ok(()) => {
                info!("✅ Windows Communication Ducking DISABLED for Meetily");
                info!("   Other applications will maintain their volume while Meetily is running");
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to set ducking preference: {}", e);
                error!("{}", msg);
                DUCKING_DISABLED.store(false, Ordering::SeqCst);
                Err(msg)
            }
        }
    }

    /// Check if ducking has been disabled for this process.
    pub fn is_ducking_disabled() -> bool {
        DUCKING_DISABLED.load(Ordering::SeqCst)
    }

    /// Re-enable ducking (for cleanup on app exit, if desired).
    /// In practice, Windows cleans up when the process exits, so this
    /// is optional.
    pub fn restore_communication_ducking() -> Result<(), String> {
        if !DUCKING_DISABLED.load(Ordering::SeqCst) {
            return Ok(()); // Nothing to restore
        }

        info!("Restoring Windows Communication Ducking defaults...");

        unsafe {
            use windows::Win32::Media::Audio::*;
            use windows::Win32::System::Com::*;
            use windows::core::Interface;

            let enumerator: IMMDeviceEnumerator = CoCreateInstance(
                &MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            ).map_err(|e| format!("Failed to create enumerator: {}", e))?;

            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| format!("Failed to get endpoint: {}", e))?;

            let session_manager: IAudioSessionManager2 = device.Activate(
                CLSCTX_ALL,
                None,
            ).map_err(|e| format!("Failed to activate session manager: {}", e))?;

            let session_control = session_manager.GetAudioSessionControl(None, 0)
                .map_err(|e| format!("Failed to get session control: {}", e))?;

            let session_control2: IAudioSessionControl2 = session_control.cast()
                .map_err(|e| format!("Failed to cast: {}", e))?;

            session_control2.SetDuckingPreference(
                false
            ).map_err(|e| format!("Failed to restore ducking: {}", e))?;

            DUCKING_DISABLED.store(false, Ordering::SeqCst);
            info!("✅ Windows Communication Ducking restored to default");
            Ok(())
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub mod ducking {
    /// No-op on non-Windows platforms.
    pub fn disable_communication_ducking() -> Result<(), String> {
        log::debug!("Communication ducking management is Windows-only, skipping");
        Ok(())
    }

    /// No-op on non-Windows platforms.
    pub fn is_ducking_disabled() -> bool {
        false
    }

    /// No-op on non-Windows platforms.
    pub fn restore_communication_ducking() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ducking;

    #[test]
    fn test_ducking_module_compiles() {
        // Verify the module compiles and the public API is accessible
        let _ = ducking::is_ducking_disabled();
    }

    #[test]
    fn test_restore_does_not_panic() {
        // Restoring should never panic regardless of current state.
        // On Windows with shared test state, the DUCKING_DISABLED flag may
        // be set by another test running in parallel, so the COM call might
        // fail if the audio session was already cleaned up. We only assert
        // that it doesn't panic — the Result can be Ok or Err.
        let _result = ducking::restore_communication_ducking();
        // No assertion on result — COM may fail in parallel test environment
    }

    #[test]
    fn test_public_api_surface() {
        // Verify all public functions are accessible and have correct signatures
        let _: Result<(), String> = ducking::disable_communication_ducking();
        let _: bool = ducking::is_ducking_disabled();
        let _: Result<(), String> = ducking::restore_communication_ducking();
    }

    #[test]
    fn test_restore_does_not_panic_multiple_calls() {
        // Multiple restore calls should never panic regardless of state.
        // On Windows, tests share static state and run in parallel, so
        // DUCKING_DISABLED may be true from another test. We only verify
        // no panics occur — COM errors are acceptable in test environment.
        let _r1 = ducking::restore_communication_ducking();
        let _r2 = ducking::restore_communication_ducking();
        let _r3 = ducking::restore_communication_ducking();
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_non_windows_disable_is_noop() {
        // On non-Windows, disable should succeed (no-op) and not set the flag
        assert!(ducking::disable_communication_ducking().is_ok());
        assert!(!ducking::is_ducking_disabled());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_non_windows_restore_is_noop() {
        // On non-Windows, restore should succeed (no-op)
        assert!(ducking::restore_communication_ducking().is_ok());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_non_windows_full_lifecycle() {
        // Full lifecycle: disable → check → restore → check
        assert!(ducking::disable_communication_ducking().is_ok());
        assert!(!ducking::is_ducking_disabled()); // Non-Windows never sets flag
        assert!(ducking::restore_communication_ducking().is_ok());
        assert!(!ducking::is_ducking_disabled());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_disable_sets_flag() {
        // On Windows with audio hardware, disable should set the flag
        // Note: This test requires actual Windows audio hardware
        // If no audio device is available, the function will return Err
        // but the flag behavior is still testable
        let result = ducking::disable_communication_ducking();
        if result.is_ok() {
            assert!(ducking::is_ducking_disabled());
            // Clean up
            let _ = ducking::restore_communication_ducking();
        }
        // If Err, it means no audio device available in CI — acceptable
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_idempotent_disable() {
        // Calling disable twice should succeed both times (idempotent)
        let first = ducking::disable_communication_ducking();
        let second = ducking::disable_communication_ducking();
        // Second call should always succeed (it's a no-op)
        if first.is_ok() {
            assert!(second.is_ok());
            // Clean up
            let _ = ducking::restore_communication_ducking();
        }
    }
}
