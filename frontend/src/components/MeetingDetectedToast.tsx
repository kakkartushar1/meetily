'use client';

import React, { useEffect, useCallback } from 'react';
import { useMicActivityMonitor } from '@/hooks/useMicActivityMonitor';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Mic } from 'lucide-react';

/**
 * MeetingDetectedToast
 *
 * Global component that listens for microphone activity detection events
 * and shows a Windows-style toast notification at the top-center of the
 * window prompting the user to start recording.
 *
 * This component should be rendered once at the app root level (e.g. in
 * layout.tsx) so it is always active regardless of the current route.
 *
 * Behaviour:
 * - Only shows when mic activity monitoring is enabled AND a meeting is detected
 * - Does NOT auto-start recording – the user must click "Start Recording"
 * - Clicking "Start Recording" navigates to the home page where recording
 *   begins via the existing autoStartRecording sessionStorage flag
 * - Clicking "Dismiss" hides the toast and tells the backend to reset
 *   the detection flag (cooldown prevents re-fire for 60 s)
 */
export function MeetingDetectedToast() {
  const {
    meetingDetected,
    isMonitoring,
    dismissDetection,
    deviceName,
  } = useMicActivityMonitor();

  const router = useRouter();

  const handleStartRecording = useCallback(() => {
    // Set the auto-start flag so the home page begins recording immediately
    sessionStorage.setItem('autoStartRecording', 'true');
    router.push('/');
    // Dismiss the detection to prevent re-showing
    dismissDetection();
    toast.dismiss('meeting-detected');
  }, [router, dismissDetection]);

  const handleDismiss = useCallback(() => {
    dismissDetection();
    toast.dismiss('meeting-detected');
  }, [dismissDetection]);

  useEffect(() => {
    if (meetingDetected && isMonitoring) {
      toast(
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <div className="flex items-center justify-center w-8 h-8 rounded-full bg-red-100 dark:bg-red-900/30">
              <Mic className="w-4 h-4 text-red-600 dark:text-red-400 animate-pulse" />
            </div>
            <div className="flex-1">
              <p className="font-semibold text-sm text-gray-900 dark:text-gray-100">
                Meeting Detected
              </p>
              <p className="text-xs text-gray-500 dark:text-gray-400">
                Microphone activity detected{deviceName ? ` on ${deviceName}` : ''}.
                Would you like to start recording?
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
              className="px-3 py-1.5 text-xs font-medium text-white bg-red-600 rounded-md hover:bg-red-700 transition-colors flex items-center gap-1"
            >
              <Mic className="w-3 h-3" />
              Start Recording
            </button>
          </div>
        </div>,
        {
          id: 'meeting-detected',
          duration: Infinity, // Stay until user acts
          position: 'top-center',
          className: 'meeting-detected-toast',
          onDismiss: () => dismissDetection(),
        }
      );
    } else {
      // If meeting is no longer detected, dismiss the toast
      toast.dismiss('meeting-detected');
    }
  }, [meetingDetected, isMonitoring, deviceName, handleStartRecording, handleDismiss, dismissDetection]);

  // This component renders nothing – it only manages toast side-effects
  return null;
}

export default MeetingDetectedToast;
