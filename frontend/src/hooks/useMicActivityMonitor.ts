import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { toast } from 'sonner';

interface MicActivityEvent {
  detected: boolean;
  rms_level: number;
  device_name: string;
  timestamp: number;
}

interface UseMicActivityMonitorReturn {
  /** Whether mic activity monitoring is currently running */
  isMonitoring: boolean;
  /** Whether a meeting has been detected (mic is actively in use) */
  meetingDetected: boolean;
  /** The user's saved preference for auto-monitoring */
  preferenceEnabled: boolean;
  /** Toggle the monitoring preference on/off */
  setPreference: (enabled: boolean) => Promise<void>;
  /** Dismiss the current detection notification */
  dismissDetection: () => Promise<void>;
  /** Name of the device being monitored */
  deviceName: string;
}

/**
 * Frontend grace period (ms) after the hook mounts before it will accept
 * meeting-detected events. This acts as a safety net on top of the Rust-side
 * grace period to prevent false-positive toasts during app startup.
 */
const FRONTEND_GRACE_PERIOD_MS = 12_000;

/**
 * Custom hook for microphone activity monitoring.
 *
 * Listens for `mic-activity-detected` and `mic-activity-stopped` events
 * from the Rust backend and exposes reactive state for the UI.
 *
 * The hook also manages the user's enable/disable preference via the
 * Tauri store and provides a method to dismiss the current detection.
 *
 * A frontend grace period suppresses detection events for the first
 * FRONTEND_GRACE_PERIOD_MS after mount to prevent false positives
 * during app startup.
 */
export function useMicActivityMonitor(): UseMicActivityMonitorReturn {
  const [isMonitoring, setIsMonitoring] = useState(true); // Default to true - enabled by default
  const [meetingDetected, setMeetingDetected] = useState(false);
  const [preferenceEnabled, setPreferenceEnabled] = useState(true); // Default to true - enabled by default
  const [deviceName, setDeviceName] = useState('');
  const unlistenDetectedRef = useRef<UnlistenFn | null>(null);
  const unlistenStoppedRef = useRef<UnlistenFn | null>(null);

  // Track when the hook was first mounted so we can enforce a grace period
  const mountTimeRef = useRef<number>(Date.now());
  // Track whether the grace period has elapsed
  const [graceElapsed, setGraceElapsed] = useState(false);

  // Start a timer that flips graceElapsed once the grace period is over
  useEffect(() => {
    const timer = setTimeout(() => {
      setGraceElapsed(true);
    }, FRONTEND_GRACE_PERIOD_MS);
    return () => clearTimeout(timer);
  }, []);

  // Load initial state
  useEffect(() => {
    const loadState = async () => {
      try {
        const [status, pref] = await Promise.all([
          invoke<boolean>('get_mic_activity_monitoring_status'),
          invoke<boolean>('get_mic_activity_monitoring_preference'),
        ]);
        setIsMonitoring(status);
        setPreferenceEnabled(pref);
      } catch (error) {
        console.error('Failed to load mic activity monitor state:', error);
      }
    };
    loadState();
  }, []);

  // Subscribe to backend events
  useEffect(() => {
    let mounted = true;

    const setup = async () => {
      // Listen for mic activity detected
      unlistenDetectedRef.current = await listen<MicActivityEvent>(
        'mic-activity-detected',
        (event) => {
          if (!mounted) return;

          // Suppress detection events during the frontend grace period
          // to prevent false-positive toasts on app startup.
          const elapsed = Date.now() - mountTimeRef.current;
          if (elapsed < FRONTEND_GRACE_PERIOD_MS) {
            console.log(
              `[MicActivityMonitor] Suppressing detection during grace period (${Math.round(elapsed / 1000)}s / ${FRONTEND_GRACE_PERIOD_MS / 1000}s)`
            );
            return;
          }

          setMeetingDetected(true);
          setDeviceName(event.payload.device_name);
        }
      );

      // Listen for mic activity stopped
      unlistenStoppedRef.current = await listen<MicActivityEvent>(
        'mic-activity-stopped',
        (event) => {
          if (!mounted) return;
          setMeetingDetected(false);
        }
      );
    };

    setup();

    return () => {
      mounted = false;
      unlistenDetectedRef.current?.();
      unlistenStoppedRef.current?.();
    };
  }, []);

  // Toggle preference
  const setPreference = useCallback(async (enabled: boolean) => {
    try {
      await invoke('set_mic_activity_monitoring_preference', { enabled });
      setPreferenceEnabled(enabled);
      setIsMonitoring(enabled);
      if (!enabled) {
        setMeetingDetected(false);
      }
      toast.success(
        enabled
          ? 'Meeting detection enabled'
          : 'Meeting detection disabled'
      );
    } catch (error) {
      console.error('Failed to set mic activity monitoring preference:', error);
      toast.error('Failed to update preference');
    }
  }, []);

  // Dismiss current detection
  const dismissDetection = useCallback(async () => {
    try {
      await invoke('dismiss_mic_activity_detection');
      setMeetingDetected(false);
    } catch (error) {
      console.error('Failed to dismiss mic activity detection:', error);
    }
  }, []);

  return {
    isMonitoring,
    meetingDetected,
    preferenceEnabled,
    setPreference,
    dismissDetection,
    deviceName,
  };
}
