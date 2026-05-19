import { toast } from 'sonner';
import Analytics from '@/lib/analytics';

/**
 * Shows the recording stopped notification toast when a call end is detected.
 * Displays a dismissible toast informing the user that recording has stopped
 * with options to:
 * - View the status of post-processing (saving, transcription)
 * - Dismiss the notification
 * - "Don't show again" checkbox
 *
 * This mirrors the pattern used in `showRecordingNotification` for recording start.
 *
 * @param meetingTitle - Optional meeting title to display in the notification
 * @returns Promise<void> - Resolves when notification is shown or skipped
 */
export async function showRecordingStoppedNotification(
  meetingTitle?: string
): Promise<void> {
  try {
    const { Store } = await import('@tauri-apps/plugin-store');
    const store = await Store.load('preferences.json');
    const showNotification =
      (await store.get<boolean>('show_recording_stopped_notification')) ?? true;

    if (!showNotification) {
      return;
    }

    let dontShowAgain = false;

    const displayTitle = meetingTitle || 'your meeting';

    const toastId = toast.info('📋 Recording Stopped', {
      description: (
        <div className="space-y-3 min-w-[280px]">
          <p className="text-sm font-medium text-gray-900">
            Call ended — recording for <strong>{displayTitle}</strong> has been
            stopped.
          </p>
          <p className="text-xs text-gray-600">
            Your transcript is being processed and will be saved automatically.
          </p>
          <label className="flex items-center gap-2 text-xs cursor-pointer hover:bg-blue-100 p-2 rounded transition-colors">
            <input
              type="checkbox"
              onChange={(e) => {
                dontShowAgain = e.target.checked;
              }}
              className="rounded border-gray-300 text-blue-600 focus:ring-blue-500 focus:ring-2"
            />
            <span className="select-none text-gray-700">
              Don't show this again
            </span>
          </label>
          <button
            onClick={async () => {
              if (dontShowAgain) {
                try {
                  const { Store } = await import('@tauri-apps/plugin-store');
                  const prefStore = await Store.load('preferences.json');
                  await prefStore.set(
                    'show_recording_stopped_notification',
                    false
                  );
                  await prefStore.save();
                } catch (error) {
                  console.error(
                    'Failed to save notification preference:',
                    error
                  );
                }
              }
              Analytics.trackButtonClick(
                'recording_stopped_notification_dismissed',
                'toast'
              );
              toast.dismiss(toastId);
            }}
            className="w-full px-3 py-1.5 bg-gray-900 text-white text-xs rounded hover:bg-gray-800 transition-colors font-medium"
          >
            Got it
          </button>
        </div>
      ),
      duration: 10000,
      position: 'bottom-right',
    });

    Analytics.trackButtonClick(
      'recording_stopped_notification_shown',
      'toast'
    );
  } catch (notificationError) {
    console.error(
      'Failed to show recording stopped notification:',
      notificationError
    );
    // Don't fail the stop flow if notification fails
  }
}
