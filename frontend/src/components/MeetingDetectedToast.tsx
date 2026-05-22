'use client';

import React, { useEffect, useCallback, useState, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useRouter, usePathname } from 'next/navigation';
import { toast } from 'sonner';
import { Video } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Analytics from '@/lib/analytics';

/**
 * MeetingDetectedToast
 *
 * Global component that listens for system-audio-started events from the
 * Rust backend's SystemAudioDetector (process monitoring for Teams/Zoom/etc.)
 * and shows a Windows-style toast notification at the top-center of the
 * window prompting the user to start recording.
 *
 * This component should be rendered once at the app root level (e.g. in
 * layout.tsx) so it is always active regardless of the current route.
 *
 * Why system audio detection instead of microphone activity:
 * - In Teams/Zoom calls, users typically keep their mic MUTED
 * - Muted mic → no microphone activity → useMicActivityMonitor never fires
 * - The SystemAudioDetector monitors running processes (teams.exe, zoom.exe,
 *   etc.) and emits `system-audio-started` when a NEW meeting app launches
 * - This reliably detects meeting START regardless of mic mute state
 *
 * Behaviour:
 * - Shows toast when a known meeting app process is newly detected
 * - Does NOT auto-start recording – the user must click "Start Recording"
 * - Clicking "Start Recording" either:
 *   a) Dispatches a custom event if already on the home page, OR
 *   b) Navigates to the home page with the autoStartRecording flag
 * - Clicking "Dismiss" hides the toast (cooldown prevents re-fire for the
 *   same session)
 * - When a meeting is detected, the window is brought to the front
 *   and set as always-on-top so the notification is visible above
 *   other windows. Always-on-top is removed when the toast is dismissed
 *   or recording starts.
 */
export function MeetingDetectedToast() {
  const [meetingDetected, setMeetingDetected] = useState(false);
  const [detectedApps, setDetectedApps] = useState<string[]>([]);

  const router = useRouter();
  const pathname = usePathname();

  // Dismissed flag: once the user dismisses for this session, don't re-show
  // until the next system-audio-started event (i.e. a new meeting starts).
  const dismissedRef = useRef(false);

  // Track whether the component has been mounted long enough to trust
  // detection events. This prevents the toast from flashing on startup
  // if the SystemAudioDetector fires during its own grace period edge cases.
  const [isReady, setIsReady] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setIsReady(true), 15_000);
    return () => clearTimeout(timer);
  }, []);

  // Helper to remove always-on-top from the window
  const removeAlwaysOnTop = useCallback(async () => {
    try {
      const appWindow = getCurrentWindow();
      await appWindow.setAlwaysOnTop(false);
    } catch (error) {
      console.error('Failed to remove always-on-top:', error);
    }
  }, []);

  // Listen for system-audio-started / system-audio-stopped events
  useEffect(() => {
    let unlistenStarted: (() => void) | undefined;
    let unlistenStopped: (() => void) | undefined;

    const setupListeners = async () => {
      try {
        // system-audio-started fires when a NEW meeting app process is detected
        unlistenStarted = await listen<string[]>('system-audio-started', (event) => {
          const apps = event.payload;
          console.log('[MeetingDetectedToast] system-audio-started detected apps:', apps);

          // Reset dismissed flag so the new meeting can show the toast
          dismissedRef.current = false;
          setDetectedApps(apps);
          setMeetingDetected(true);
        });

        // system-audio-stopped fires when all meeting app processes have exited
        unlistenStopped = await listen('system-audio-stopped', () => {
          console.log('[MeetingDetectedToast] system-audio-stopped — hiding toast');
          setMeetingDetected(false);
          toast.dismiss('meeting-detected');
        });

        console.log('[MeetingDetectedToast] Event listeners setup complete');
      } catch (error) {
        console.error('[MeetingDetectedToast] Failed to setup event listeners:', error);
      }
    };

    setupListeners();

    return () => {
      if (unlistenStarted) unlistenStarted();
      if (unlistenStopped) unlistenStopped();
      console.log('[MeetingDetectedToast] Event listeners cleaned up');
    };
  }, []);

  const handleStartRecording = useCallback(() => {
    // Dismiss the toast first
    dismissedRef.current = true;
    setMeetingDetected(false);
    toast.dismiss('meeting-detected');
    removeAlwaysOnTop();

    try {
      Analytics.trackButtonClick('meeting_detected_start_recording', 'meeting_detected_toast');
    } catch (e) {
      // Analytics failure should never block recording
    }

    if (pathname === '/') {
      // Already on the home page — dispatch a custom event so the
      // useRecordingStart hook picks it up directly without needing
      // a navigation (router.push('/') is a no-op on the same route).
      sessionStorage.setItem('autoStartRecording', 'true');
      window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
    } else {
      // Not on home — navigate and use the auto-start mechanism
      sessionStorage.setItem('autoStartRecording', 'true');
      router.push('/');
    }
  }, [router, pathname, removeAlwaysOnTop]);

  const handleDismiss = useCallback(() => {
    dismissedRef.current = true;
    setMeetingDetected(false);
    toast.dismiss('meeting-detected');
    removeAlwaysOnTop();

    try {
      Analytics.trackButtonClick('meeting_detected_dismiss', 'meeting_detected_toast');
    } catch (e) {
      // Analytics failure should never block dismiss
    }
    console.log('[MeetingDetectedToast] Toast dismissed by user');
  }, [removeAlwaysOnTop]);

  // Bring the window to the front and set always-on-top when a meeting is detected
  useEffect(() => {
    if (meetingDetected && isReady && !dismissedRef.current) {
      const bringToFront = async () => {
        try {
          const appWindow = getCurrentWindow();
          await appWindow.setAlwaysOnTop(true);
          await appWindow.setFocus();
        } catch (error) {
          console.error('Failed to set always-on-top:', error);
        }
      };
      bringToFront();
    } else {
      // When meeting is no longer detected, remove always-on-top
      removeAlwaysOnTop();
    }
  }, [meetingDetected, isReady, removeAlwaysOnTop]);

  // Show or dismiss the toast based on detection state
  useEffect(() => {
    if (meetingDetected && isReady && !dismissedRef.current) {
      // Build a friendly description of which app was detected
      const appLabel =
        detectedApps.length > 0
          ? detectedApps.join(', ')
          : 'a meeting app';

      toast(
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <div className="flex items-center justify-center w-8 h-8 rounded-full bg-blue-100 dark:bg-blue-900/30">
              <Video className="w-4 h-4 text-blue-600 dark:text-blue-400 animate-pulse" />
            </div>
            <div className="flex-1">
              <p className="font-semibold text-sm text-gray-900 dark:text-gray-100">
                Meeting Detected
              </p>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                {appLabel} is running. Would you like to start recording?
              </p>
            </div>
          </div>
          <div className="flex gap-2 justify-end">
            <button
              onClick={handleDismiss}
              className="px-3 py-1.5 text-xs font-medium text-gray-600 dark:text-gray-400 bg-gray-100 dark:bg-gray-800 rounded-md hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
            >
              Dismiss
            </button>
            <button
              onClick={handleStartRecording}
              className="px-3 py-1.5 text-xs font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 transition-colors flex items-center gap-1"
            >
              <Video className="w-3 h-3" />
              Start Recording
            </button>
          </div>
        </div>,
        {
          id: 'meeting-detected',
          duration: Infinity, // Stay until user acts
          position: 'top-center',
          className: 'meeting-detected-toast',
          onDismiss: () => {
            dismissedRef.current = true;
            setMeetingDetected(false);
            removeAlwaysOnTop();
          },
        }
      );
    } else if (!meetingDetected || dismissedRef.current) {
      // If meeting is no longer detected or was dismissed, hide the toast
      toast.dismiss('meeting-detected');
    }
  }, [meetingDetected, isReady, detectedApps, handleStartRecording, handleDismiss, removeAlwaysOnTop]);

  // This component renders nothing – it only manages toast side-effects
  return null;
}

export default MeetingDetectedToast;
