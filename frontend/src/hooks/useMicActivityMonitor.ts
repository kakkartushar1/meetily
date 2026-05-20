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
 * Custom hook for microphone activity monitoring.
 *
 * Listens for `mic-activity-detected` and `mic-activity-stopped` events
 * from the Rust backend and exposes reactive state for the UI.
 *
 * The hook also manages the user's enable/disable preference via the
 * Tauri store and provides a method to dismiss the current detection.
 */
export function useMicActivityMonitor(): UseMicActivityMonitorReturn {
  const [isMonitoring, setIsMonitoring] = useState(false);
  const [meetingDetected, setMeetingDetected] = useState(false);
  const [preferenceEnabled, setPreferenceEnabled] = useState(false);
  const [deviceName, setDeviceName] = useState('');
  const unlistenDetectedRef = useRef<UnlistenFn | null>(null);
  const unlistenStoppedRef = useRef<UnlistenFn | null>(null);

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
