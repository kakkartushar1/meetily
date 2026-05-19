'use client';

import React, { useCallback } from 'react';
import { PhoneOff, Mic } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import Analytics from '@/lib/analytics';

interface CallEndedDialogProps {
  /** Whether the dialog is open */
  isOpen: boolean;
  /** Callback when the user chooses to stop recording */
  onStopRecording: () => void;
  /** Callback when the user chooses to continue recording */
  onContinueRecording: () => void;
  /** Optional: detected apps that stopped audio (for context) */
  lastDetectedApps?: string[];
}

/**
 * Dialog shown when system audio stops during an active recording.
 * Prompts the user to either stop or continue recording.
 *
 * This is triggered by the `system-audio-stopped` event from the
 * Rust backend's SystemAudioDetector when a meeting call ends.
 */
export const CallEndedDialog: React.FC<CallEndedDialogProps> = ({
  isOpen,
  onStopRecording,
  onContinueRecording,
  lastDetectedApps,
}) => {
  const handleStopRecording = useCallback(() => {
    Analytics.trackButtonClick('call_ended_stop_recording', 'call_ended_dialog');
    onStopRecording();
  }, [onStopRecording]);

  const handleContinueRecording = useCallback(() => {
    Analytics.trackButtonClick('call_ended_continue_recording', 'call_ended_dialog');
    onContinueRecording();
  }, [onContinueRecording]);

  // Build a user-friendly message about which app stopped
  const appContext = lastDetectedApps && lastDetectedApps.length > 0
    ? `It looks like ${lastDetectedApps.join(', ')} has ended.`
    : 'It looks like your meeting call has ended.';

  return (
    <Dialog open={isOpen} onOpenChange={(open) => { if (!open) handleContinueRecording(); }}>
      <DialogContent
        className="sm:max-w-md"
        aria-describedby="call-ended-description"
        onPointerDownOutside={(e) => e.preventDefault()}
        onEscapeKeyDown={(e) => e.preventDefault()}
      >
        <DialogHeader>
          <div className="flex items-center gap-3 mb-2">
            <div className="flex h-10 w-10 items-center justify-center rounded-full bg-orange-100">
              <PhoneOff className="h-5 w-5 text-orange-600" aria-hidden="true" />
            </div>
            <DialogTitle className="text-lg font-semibold">
              Meeting Call Ended
            </DialogTitle>
          </div>
          <DialogDescription id="call-ended-description" className="text-sm text-gray-600">
            {appContext} Would you like to stop recording and save your meeting, or continue recording?
          </DialogDescription>
        </DialogHeader>

        <DialogFooter className="flex flex-col-reverse sm:flex-row sm:justify-end gap-2 mt-4">
          <Button
            variant="outline"
            onClick={handleContinueRecording}
            className="flex items-center gap-2"
            aria-label="Continue recording"
          >
            <Mic className="h-4 w-4" aria-hidden="true" />
            Continue Recording
          </Button>
          <Button
            variant="default"
            onClick={handleStopRecording}
            className="flex items-center gap-2 bg-red-500 hover:bg-red-600 text-white"
            aria-label="Stop recording and save meeting"
          >
            <PhoneOff className="h-4 w-4" aria-hidden="true" />
            Stop & Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
