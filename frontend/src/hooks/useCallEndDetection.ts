import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import Analytics from '@/lib/analytics';

interface UseCallEndDetectionReturn {
  /** Whether the call-ended dialog should be shown */
  showCallEndedDialog: boolean;
  /** Last detected apps that were using system audio before it stopped */
  lastDetectedApps: string[];
  /** Handler to stop recording (user chose to stop) */
  handleStopFromCallEnd: () => void;
  /** Handler to continue recording (user chose to continue) */
  handleContinueFromCallEnd: () => void;
}

/**
 * Custom hook for detecting when a meeting call ends during active recording.
 *
 * Listens for `system-audio-stopped` events from the Rust backend's
 * SystemAudioDetector. When system audio stops while recording is active,
 * it shows a dialog asking the user if they want to stop recording.
 *
 * Also tracks `system-audio-started` events to know which apps were
 * using system audio (e.g., Zoom, Teams, Google Meet) for context.
 *
 * Features:
 * - Debounced detection (3s cooldown) to avoid false positives
 * - Tracks last known apps using system audio
 * - Only triggers during active recording sessions
 * - Auto-dismisses if recording stops from another source
 * - Analytics tracking for user decisions
 */
export function useCallEndDetection(
  onStopRecording: () => void
): UseCallEndDetectionReturn {
  const [showCallEndedDialog, setShowCallEndedDialog] = useState(false);
  const [lastDetectedApps, setLastDetectedApps] = useState<string[]>([]);

  const recordingState = useRecordingState();
  const lastAudioStopTimeRef = useRef<number>(0);
  const dismissedRef = useRef<boolean>(false);

  // Known meeting apps for filtering relevant system audio events
  const MEETING_APPS = [
    'zoom', 'zoom.us', 'microsoft teams', 'teams',
    'google meet', 'google chrome', 'slack', 'discord',
    'webex', 'cisco webex', 'skype', 'facetime',
    'brave browser', 'firefox', 'safari', 'arc',
    'microsoft edge',
  ];

  /**
   * Check if any of the detected apps are known meeting/communication apps.
   * This helps reduce false positives from non-meeting audio.
   */
  const hasMeetingApp = useCallback((apps: string[]): boolean => {
    return apps.some(app =>
      MEETING_APPS.some(meetingApp =>
        app.toLowerCase().includes(meetingApp.toLowerCase())
      )
    );
  }, []);

  // Auto-dismiss dialog if recording stops from another source
  useEffect(() => {
    if (!recordingState.isRecording && showCallEndedDialog) {
      setShowCallEndedDialog(false);
    }
  }, [recordingState.isRecording, showCallEndedDialog]);

  // Reset dismissed flag when a new recording starts
  useEffect(() => {
    if (recordingState.isRecording) {
      dismissedRef.current = false;
    }
  }, [recordingState.isRecording]);

  // Listen for system audio events
  useEffect(() => {
    let unlistenStarted: (() => void) | undefined;
    let unlistenStopped: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        // Track which apps are using system audio
        unlistenStarted = await listen<string[]>('system-audio-started', (event) => {
          const apps = event.payload;
          console.log('[CallEndDetection] System audio started by apps:', apps);
          setLastDetectedApps(apps);
        });

        // Detect when system audio stops during recording
        unlistenStopped = await listen('system-audio-stopped', () => {
          const now = Date.now();
          const timeSinceLastStop = now - lastAudioStopTimeRef.current;

          console.log('[CallEndDetection] System audio stopped event received', {
            isRecording: recordingState.isRecording,
            timeSinceLastStop,
            lastDetectedApps,
            dismissed: dismissedRef.current,
          });

          // Only show dialog if:
          // 1. Recording is active
          // 2. Not already showing the dialog
          // 3. Debounce: at least 3 seconds since last stop event
          // 4. User hasn't already dismissed the dialog for this recording session
          // 5. We previously detected meeting-related apps
          if (
            recordingState.isRecording &&
            !showCallEndedDialog &&
            timeSinceLastStop > 3000 &&
            !dismissedRef.current &&
            hasMeetingApp(lastDetectedApps)
          ) {
            lastAudioStopTimeRef.current = now;
            setShowCallEndedDialog(true);

            Analytics.trackButtonClick('call_ended_detected', 'system_audio');
            console.log('[CallEndDetection] Showing call-ended dialog');
          } else {
            console.log('[CallEndDetection] Suppressed call-ended dialog', {
              reason: !recordingState.isRecording
                ? 'not recording'
                : showCallEndedDialog
                  ? 'already showing'
                  : timeSinceLastStop <= 3000
                    ? 'debounce'
                    : dismissedRef.current
                      ? 'dismissed'
                      : 'no meeting app detected',
            });
          }
        });

        console.log('[CallEndDetection] Event listeners setup complete');
      } catch (error) {
        console.error('[CallEndDetection] Failed to setup event listeners:', error);
      }
    };

    setupListeners();

    return () => {
      if (unlistenStarted) unlistenStarted();
      if (unlistenStopped) unlistenStopped();
      console.log('[CallEndDetection] Event listeners cleaned up');
    };
  }, [recordingState.isRecording, showCallEndedDialog, lastDetectedApps, hasMeetingApp]);

  // Handle user choosing to stop recording
  const handleStopFromCallEnd = useCallback(() => {
    console.log('[CallEndDetection] User chose to stop recording');
    setShowCallEndedDialog(false);
    onStopRecording();
  }, [onStopRecording]);

  // Handle user choosing to continue recording
  const handleContinueFromCallEnd = useCallback(() => {
    console.log('[CallEndDetection] User chose to continue recording');
    setShowCallEndedDialog(false);
    dismissedRef.current = true; // Don't show again for this recording session
  }, []);

  return {
    showCallEndedDialog,
    lastDetectedApps,
    handleStopFromCallEnd,
    handleContinueFromCallEnd,
  };
}
