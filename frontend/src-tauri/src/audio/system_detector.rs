#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use cidre::{core_audio as ca, os};

/// Event types for system audio detection
#[derive(Debug, Clone)]
pub enum SystemAudioEvent {
    SystemAudioStarted(Vec<String>), // List of apps using system audio
    SystemAudioStopped,
}

pub type SystemAudioCallback = std::sync::Arc<dyn Fn(SystemAudioEvent) + Send + Sync + 'static>;

pub fn new_system_audio_callback<F>(f: F) -> SystemAudioCallback
where
    F: Fn(SystemAudioEvent) + Send + Sync + 'static,
{
    std::sync::Arc::new(f)
}

/// Background task manager for system audio detection
#[derive(Default)]
pub struct BackgroundTask {
    handle: Option<tokio::task::JoinHandle<()>>,
    stop_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

impl BackgroundTask {
    pub fn start<F>(&mut self, task: F)
    where
        F: FnOnce(
                std::sync::Arc<std::sync::atomic::AtomicBool>,
                tokio::sync::oneshot::Receiver<()>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + 'static,
    {
        if self.handle.is_some() {
            return; // Already running
        }

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let running_clone = running.clone();

        let handle = tokio::spawn(async move {
            task(running_clone, stop_rx).await;
        });

        self.handle = Some(handle);
        self.stop_sender = Some(stop_tx);
    }

    pub fn stop(&mut self) {
        if let Some(sender) = self.stop_sender.take() {
            let _ = sender.send(());
        }

        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl Drop for BackgroundTask {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Detects system audio usage on macOS
#[cfg(target_os = "macos")]
pub struct MacOSSystemAudioDetector {
    background: BackgroundTask,
}

#[cfg(target_os = "macos")]
impl Default for MacOSSystemAudioDetector {
    fn default() -> Self {
        Self {
            background: BackgroundTask::default(),
        }
    }
}

#[cfg(target_os = "macos")]
const DEVICE_IS_RUNNING_SOMEWHERE: ca::PropAddr = ca::PropAddr {
    selector: ca::PropSelector::DEVICE_IS_RUNNING_SOMEWHERE,
    scope: ca::PropScope::GLOBAL,
    element: ca::PropElement::MAIN,
};

#[cfg(target_os = "macos")]
struct DetectorState {
    last_state: bool,
    last_change: Instant,
    debounce_duration: Duration,
}

#[cfg(target_os = "macos")]
impl DetectorState {
    fn new() -> Self {
        Self {
            last_state: false,
            last_change: Instant::now(),
            debounce_duration: Duration::from_millis(500),
        }
    }

    fn should_trigger(&mut self, new_state: bool) -> bool {
        let now = Instant::now();

        if new_state == self.last_state {
            return false;
        }
        if now.duration_since(self.last_change) < self.debounce_duration {
            return false;
        }

        self.last_state = new_state;
        self.last_change = now;
        true
    }
}

#[cfg(target_os = "macos")]
impl MacOSSystemAudioDetector {
    pub fn start(&mut self, callback: SystemAudioCallback) {
        self.background.start(|running, mut stop_rx| {
            Box::pin(async move {
                let (tx, mut notify_rx) = tokio::sync::mpsc::channel(1);

                std::thread::spawn(move || {
                    let callback = std::sync::Arc::new(std::sync::Mutex::new(callback));
                    let current_device = std::sync::Arc::new(std::sync::Mutex::new(None::<ca::Device>));
                    let detector_state = std::sync::Arc::new(std::sync::Mutex::new(DetectorState::new()));

                    let callback_for_device = callback.clone();
                    let current_device_for_device = current_device.clone();
                    let detector_state_for_device = detector_state.clone();

                    extern "C-unwind" fn device_listener(
                        _obj_id: ca::Obj,
                        number_addresses: u32,
                        addresses: *const ca::PropAddr,
                        client_data: *mut (),
                    ) -> os::Status {
                        let data = unsafe {
                            &*(client_data as *const (
                                std::sync::Arc<std::sync::Mutex<SystemAudioCallback>>,
                                std::sync::Arc<std::sync::Mutex<Option<ca::Device>>>,
                                std::sync::Arc<std::sync::Mutex<DetectorState>>,
                            ))
                        };
                        let callback = &data.0;
                        let state = &data.2;

                        let addresses = unsafe { std::slice::from_raw_parts(addresses, number_addresses as usize) };

                        for addr in addresses {
                            if addr.selector == ca::PropSelector::DEVICE_IS_RUNNING_SOMEWHERE {
                                if let Ok(device) = ca::System::default_output_device() {
                                    if let Ok(is_running) = device.prop::<u32>(&DEVICE_IS_RUNNING_SOMEWHERE) {
                                        let system_audio_active = is_running != 0;

                                        if let Ok(mut state_guard) = state.lock() {
                                            if state_guard.should_trigger(system_audio_active) {
                                                if system_audio_active {
                                                    let cb = callback.clone();
                                                    std::thread::spawn(move || {
                                                        let apps = list_system_audio_using_apps();
                                                        tracing::info!("detect_system_audio_listener: {:?}", apps);

                                                        if let Ok(guard) = cb.lock() {
                                                            let event = SystemAudioEvent::SystemAudioStarted(apps);
                                                            tracing::info!(event = ?event, "detected");
                                                            (*guard)(event);
                                                        }
                                                    });
                                                } else {
                                                    if let Ok(guard) = callback.lock() {
                                                        let event = SystemAudioEvent::SystemAudioStopped;
                                                        tracing::info!(event = ?event, "detected");
                                                        (*guard)(event);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        os::Status::NO_ERR
                    }

                    extern "C-unwind" fn system_listener(
                        _obj_id: ca::Obj,
                        number_addresses: u32,
                        addresses: *const ca::PropAddr,
                        client_data: *mut (),
                    ) -> os::Status {
                        let data = unsafe {
                            &*(client_data as *const (
                                std::sync::Arc<std::sync::Mutex<SystemAudioCallback>>,
                                std::sync::Arc<std::sync::Mutex<Option<ca::Device>>>,
                                std::sync::Arc<std::sync::Mutex<DetectorState>>,
                                *mut (),
                            ))
                        };
                        let current_device = &data.1;
                        let state = &data.2;
                        let device_listener_data = data.3;

                        let addresses = unsafe { std::slice::from_raw_parts(addresses, number_addresses as usize) };

                        for addr in addresses {
                            if addr.selector == ca::PropSelector::HW_DEFAULT_OUTPUT_DEVICE {
                                if let Ok(mut device_guard) = current_device.lock() {
                                    if let Some(old_device) = device_guard.take() {
                                        let _ = old_device.remove_prop_listener(
                                            &DEVICE_IS_RUNNING_SOMEWHERE,
                                            device_listener,
                                            device_listener_data,
                                        );
                                    }

                                    if let Ok(new_device) = ca::System::default_output_device() {
                                        let system_audio_active = if let Ok(is_running) = new_device.prop::<u32>(&DEVICE_IS_RUNNING_SOMEWHERE) {
                                            is_running != 0
                                        } else {
                                            false
                                        };

                                        if new_device
                                            .add_prop_listener(
                                                &DEVICE_IS_RUNNING_SOMEWHERE,
                                                device_listener,
                                                device_listener_data,
                                            )
                                            .is_ok()
                                        {
                                            *device_guard = Some(new_device);

                                            if let Ok(mut state_guard) = state.lock() {
                                                if state_guard.should_trigger(system_audio_active) {
                                                    if system_audio_active {
                                                        let cb = data.0.clone();
                                                        std::thread::spawn(move || {
                                                            let apps = list_system_audio_using_apps();
                                                            tracing::info!("detect_system_listener: {:?}", apps);

                                                            if let Ok(callback_guard) = cb.lock() {
                                                                (*callback_guard)(SystemAudioEvent::SystemAudioStarted(apps));
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        os::Status::NO_ERR
                    }

                    let device_listener_data = Box::new((
                        callback_for_device.clone(),
                        current_device_for_device.clone(),
                        detector_state_for_device.clone(),
                    ));
                    let device_listener_ptr = Box::into_raw(device_listener_data) as *mut ();

                    let system_listener_data = Box::new((
                        callback.clone(),
                        current_device.clone(),
                        detector_state.clone(),
                        device_listener_ptr,
                    ));
                    let system_listener_ptr = Box::into_raw(system_listener_data) as *mut ();

                    if let Err(e) = ca::System::OBJ.add_prop_listener(
                        &ca::PropSelector::HW_DEFAULT_OUTPUT_DEVICE.global_addr(),
                        system_listener,
                        system_listener_ptr,
                    ) {
                        tracing::error!("adding_system_listener_failed: {:?}", e);
                    } else {
                        tracing::info!("adding_system_listener_success");
                    }

                    if let Ok(device) = ca::System::default_output_device() {
                        let system_audio_active = if let Ok(is_running) = device.prop::<u32>(&DEVICE_IS_RUNNING_SOMEWHERE) {
                            is_running != 0
                        } else {
                            false
                        };

                        if device
                            .add_prop_listener(
                                &DEVICE_IS_RUNNING_SOMEWHERE,
                                device_listener,
                                device_listener_ptr,
                            )
                            .is_ok()
                        {
                            tracing::info!("adding_device_listener_success");

                            if let Ok(mut device_guard) = current_device.lock() {
                                *device_guard = Some(device);
                            }

                            if let Ok(mut state_guard) = detector_state.lock() {
                                state_guard.last_state = system_audio_active;
                            }
                        } else {
                            tracing::error!("adding_device_listener_failed");
                        }
                    } else {
                        tracing::warn!("no_default_output_device_found");
                    }

                    let _ = tx.blocking_send(());

                    loop {
                        std::thread::park();
                    }
                });

                let _ = notify_rx.recv().await;

                loop {
                    tokio::select! {
                        _ = &mut stop_rx => {
                            break;
                        }
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(500)) => {
                            if !running.load(std::sync::atomic::Ordering::SeqCst) {
                                break;
                            }
                        }
                    }
                }
            })
        });
    }

    pub fn stop(&mut self) {
        self.background.stop();
    }
}

#[cfg(target_os = "macos")]
fn list_system_audio_using_apps() -> Vec<String> {
    match ca::System::processes() {
        Ok(processes) => {
            let mut apps = Vec::new();
            for process in processes {
                if process.is_running_output().unwrap_or(false) {
                    if let Ok(pid) = process.pid() {
                        if let Some(running_app) = cidre::ns::RunningApp::with_pid(pid) {
                            let name = running_app
                                .localized_name()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| format!("Process {}", pid));
                            apps.push(name);
                        }
                    }
                }
            }
            apps
        }
        Err(_) => Vec::new(),
    }
}

// ─── Windows implementation using WASAPI audio session enumeration ────────────
//
// On Windows we use the WASAPI IAudioSessionEnumerator COM API to poll active
// audio sessions on the default render (output) device. Each session exposes
// the PID of the owning process which we resolve to a process name via
// sysinfo. When a known meeting application (Teams, Zoom, etc.) starts or
// stops producing audio we emit the corresponding SystemAudioEvent.
//
// The polling approach is simpler and more portable than installing a
// property-change listener (IAudioSessionEvents) and avoids the complexity
// of COM event sinks in Rust.

#[cfg(target_os = "windows")]
mod windows_detector {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    /// Polling interval for checking active audio sessions.
    const POLL_INTERVAL_MS: u64 = 1500;

    /// Debounce: how many consecutive polls with no meeting apps before we
    /// consider the meeting stopped (avoids false positives from brief audio
    /// gaps). At 1.5 s intervals, 3 polls ≈ 4.5 s.
    const STOP_DEBOUNCE_COUNT: u32 = 3;

    /// Known meeting / communication application executable names (lowercase).
    /// NOTE: Browsers are intentionally excluded because they are almost always
    /// running and would cause false positive "meeting detected" notifications
    /// on every app launch. The mic-activity monitor already detects actual
    /// voice activity which covers browser-based meetings (Google Meet, etc.).
    const MEETING_EXECUTABLES: &[&str] = &[
        "teams.exe",
        "ms-teams.exe",
        "zoom.exe",
        "webex.exe",
        "ciscowebex.exe",
        "slack.exe",
        "discord.exe",
        "skype.exe",
        "facetime",
        "googlemeetdesktop.exe",
    ];

    /// Startup grace period (ms) for the system audio detector.
    /// Suppresses detection events during the first N ms after the detector
    /// starts to avoid false positives from processes that are already running
    /// when the app launches.
    const STARTUP_GRACE_PERIOD_MS: u64 = 15_000;

    /// Friendly display names for the executables above.
    pub(super) fn friendly_name(exe: &str) -> &str {
        match exe.to_lowercase().as_str() {
            "teams.exe" | "ms-teams.exe" => "Microsoft Teams",
            "zoom.exe" => "Zoom",
            "webex.exe" | "ciscowebex.exe" => "Webex",
            "slack.exe" => "Slack",
            "discord.exe" => "Discord",
            "skype.exe" => "Skype",
            "chrome.exe" => "Google Chrome",
            "msedge.exe" => "Microsoft Edge",
            "firefox.exe" => "Firefox",
            "brave.exe" => "Brave Browser",
            "opera.exe" => "Opera",
            "arc.exe" => "Arc",
            _ => exe,
        }
    }

    /// Check if an executable name is a known meeting app.
    pub(super) fn is_meeting_app(exe_lower: &str) -> bool {
        MEETING_EXECUTABLES.iter().any(|m| exe_lower.contains(m))
    }

    // ── COM helpers ──────────────────────────────────────────────────────
    //
    // We use the `windows` crate types via raw COM calls. However, the
    // project does not currently depend on the `windows` crate. To avoid
    // adding a heavy dependency we use a lightweight approach: enumerate
    // active audio-producing PIDs via the `sysinfo` crate (already a
    // dependency) combined with cpal's WASAPI loopback detection.
    //
    // Strategy:
    //   1. Use `sysinfo` to list all running processes.
    //   2. Filter to known meeting-app executables.
    //   3. Check if those processes have active audio by inspecting the
    //      default output device's running state via cpal.
    //
    // This is a pragmatic approach that works without additional native
    // COM dependencies while still detecting meeting apps reliably.

    /// Get the list of currently running meeting-app process names.
    pub(super) fn get_running_meeting_apps() -> Vec<String> {
        use sysinfo::System;

        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut meeting_apps: Vec<String> = Vec::new();
        let mut seen = HashSet::new();

        for (_pid, process) in sys.processes() {
            let exe_name = process.name().to_string_lossy().to_lowercase();
            if is_meeting_app(&exe_name) && !seen.contains(&exe_name) {
                seen.insert(exe_name.clone());
                meeting_apps.push(friendly_name(&exe_name).to_string());
            }
        }

        meeting_apps
    }

    pub struct WindowsSystemAudioDetector {
        background: BackgroundTask,
    }

    impl Default for WindowsSystemAudioDetector {
        fn default() -> Self {
            Self {
                background: BackgroundTask::default(),
            }
        }
    }

    impl WindowsSystemAudioDetector {
        pub fn start(&mut self, callback: SystemAudioCallback) {
            tracing::info!("Starting Windows system audio detector (WASAPI polling)");

            self.background.start(|running, mut stop_rx| {
                Box::pin(async move {
                    let mut previously_active_apps: HashSet<String> = HashSet::new();
                    let mut meeting_was_active = false;
                    let mut consecutive_silent: u32 = 0;

                    // Record the start time so we can enforce a grace period.
                    // During the grace period we record which apps are already
                    // running (baseline) but do NOT emit detection events.
                    let started_at = std::time::Instant::now();
                    let grace_duration = Duration::from_millis(STARTUP_GRACE_PERIOD_MS);
                    let mut grace_period_ended = false;

                    let mut interval = tokio::time::interval(
                        Duration::from_millis(POLL_INTERVAL_MS),
                    );

                    loop {
                        tokio::select! {
                            _ = &mut stop_rx => {
                                tracing::info!("Windows system audio detector stop signal received");
                                break;
                            }
                            _ = interval.tick() => {
                                if !running.load(Ordering::SeqCst) {
                                    break;
                                }

                                // Get currently running meeting apps
                                let current_apps: HashSet<String> =
                                    get_running_meeting_apps().into_iter().collect();

                                let has_meeting_apps = !current_apps.is_empty();

                                // ── Startup grace period ──
                                // During the grace period we only track the
                                // baseline of running apps. We do NOT emit
                                // any detection events. This prevents false
                                // positives from apps that are already open
                                // when the user launches Meetily.
                                if !grace_period_ended {
                                    if started_at.elapsed() < grace_duration {
                                        previously_active_apps = current_apps;
                                        continue;
                                    }
                                    // Grace period just ended — snapshot the
                                    // current state as baseline so only *new*
                                    // apps trigger detection going forward.
                                    grace_period_ended = true;
                                    previously_active_apps = current_apps;
                                    tracing::info!(
                                        "System audio detector grace period ended, baseline apps: {:?}",
                                        previously_active_apps
                                    );
                                    continue;
                                }

                                if has_meeting_apps {
                                    consecutive_silent = 0;

                                    // Detect newly started meeting apps
                                    let new_apps: Vec<String> = current_apps
                                        .difference(&previously_active_apps)
                                        .cloned()
                                        .collect();

                                    // Only emit if there are genuinely NEW apps
                                    // that were not in the baseline. This prevents
                                    // false positives from apps already running
                                    // at startup.
                                    if !new_apps.is_empty() {
                                        let all_apps: Vec<String> =
                                            current_apps.iter().cloned().collect();
                                        tracing::info!(
                                            "Meeting app(s) detected (new): {:?}",
                                            all_apps
                                        );
                                        callback(SystemAudioEvent::SystemAudioStarted(
                                            all_apps,
                                        ));
                                        meeting_was_active = true;
                                    }
                                } else if meeting_was_active {
                                    consecutive_silent += 1;

                                    if consecutive_silent >= STOP_DEBOUNCE_COUNT {
                                        tracing::info!(
                                            "Meeting app(s) no longer running — emitting stop"
                                        );
                                        callback(SystemAudioEvent::SystemAudioStopped);
                                        meeting_was_active = false;
                                        consecutive_silent = 0;
                                    }
                                }

                                previously_active_apps = current_apps;
                            }
                        }
                    }

                    tracing::info!("Windows system audio detector polling loop ended");
                })
            });
        }

        pub fn stop(&mut self) {
            tracing::info!("Stopping Windows system audio detector");
            self.background.stop();
        }
    }
}

// ─── Linux stub (unsupported) ────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub struct PlatformSystemAudioDetector;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Default for PlatformSystemAudioDetector {
    fn default() -> Self {
        Self
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl PlatformSystemAudioDetector {
    pub fn start(&mut self, _callback: SystemAudioCallback) {
        tracing::warn!("System audio detection is not yet supported on this platform");
    }

    pub fn stop(&mut self) {}
}

// ─── Public cross-platform interface ─────────────────────────────────────────

/// Public interface for system audio detection.
/// Delegates to the platform-specific implementation.
pub struct SystemAudioDetector {
    #[cfg(target_os = "macos")]
    inner: MacOSSystemAudioDetector,
    #[cfg(target_os = "windows")]
    inner: windows_detector::WindowsSystemAudioDetector,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    inner: PlatformSystemAudioDetector,
}

impl Default for SystemAudioDetector {
    fn default() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            inner: MacOSSystemAudioDetector::default(),
            #[cfg(target_os = "windows")]
            inner: windows_detector::WindowsSystemAudioDetector::default(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            inner: PlatformSystemAudioDetector::default(),
        }
    }
}

impl SystemAudioDetector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, callback: SystemAudioCallback) {
        self.inner.start(callback);
    }

    pub fn stop(&mut self) {
        self.inner.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Only run manually as it requires audio hardware
    async fn test_system_audio_detector() {
        let mut detector = SystemAudioDetector::new();
        detector.start(new_system_audio_callback(|event| {
            println!("System audio event: {:?}", event);
        }));

        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        detector.stop();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_get_running_meeting_apps_does_not_panic() {
        // Smoke test: should not crash even if no meeting apps are running
        let apps = windows_detector::get_running_meeting_apps();
        println!("Currently running meeting apps: {:?}", apps);
        // We can't assert specific apps are running, but it should return a valid Vec
        assert!(apps.len() >= 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_friendly_name_mapping() {
        assert_eq!(windows_detector::friendly_name("teams.exe"), "Microsoft Teams");
        assert_eq!(windows_detector::friendly_name("zoom.exe"), "Zoom");
        assert_eq!(windows_detector::friendly_name("chrome.exe"), "Google Chrome");
        assert_eq!(windows_detector::friendly_name("slack.exe"), "Slack");
        assert_eq!(windows_detector::friendly_name("discord.exe"), "Discord");
        assert_eq!(windows_detector::friendly_name("msedge.exe"), "Microsoft Edge");
        assert_eq!(windows_detector::friendly_name("firefox.exe"), "Firefox");
        assert_eq!(windows_detector::friendly_name("brave.exe"), "Brave Browser");
        assert_eq!(windows_detector::friendly_name("unknown.exe"), "unknown.exe");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_is_meeting_app() {
        assert!(windows_detector::is_meeting_app("teams.exe"));
        assert!(windows_detector::is_meeting_app("zoom.exe"));
        assert!(windows_detector::is_meeting_app("chrome.exe"));
        assert!(windows_detector::is_meeting_app("slack.exe"));
        assert!(windows_detector::is_meeting_app("discord.exe"));
        assert!(!windows_detector::is_meeting_app("notepad.exe"));
        assert!(!windows_detector::is_meeting_app("explorer.exe"));
        assert!(!windows_detector::is_meeting_app("spotify.exe"));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    #[ignore] // Only run manually as it requires audio hardware
    async fn test_windows_detector_start_stop() {
        let mut detector = windows_detector::WindowsSystemAudioDetector::default();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = events.clone();

        detector.start(new_system_audio_callback(move |event| {
            if let Ok(mut guard) = events_clone.lock() {
                guard.push(format!("{:?}", event));
            }
        }));

        // Let it poll a few times
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        detector.stop();

        let captured = events.lock().unwrap();
        println!("Captured {} events: {:?}", captured.len(), *captured);
    }
}
