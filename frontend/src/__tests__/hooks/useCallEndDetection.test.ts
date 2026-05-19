import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useCallEndDetection } from '@/hooks/useCallEndDetection';

// ── Type definitions for event listeners ──────────────────────────────

type EventCallback = (event: { payload: unknown }) => void;

interface ListenerRegistration {
  event: string;
  callback: EventCallback;
}

// ── Mocks ─────────────────────────────────────────────────────────────

// Store ALL registered Tauri event listeners so tests can trigger them.
let registeredListeners: ListenerRegistration[] = [];

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, callback: EventCallback) => {
    registeredListeners.push({ event, callback });
    // Return an unlisten function
    return () => {
      registeredListeners = registeredListeners.filter(
        (l) => !(l.event === event && l.callback === callback)
      );
    };
  }),
}));

// Mock RecordingStateContext - use a mutable object so tests can change values
const mockRecordingState = {
  isRecording: false,
  isPaused: false,
  isActive: false,
  recordingDuration: null,
  activeDuration: null,
  status: 'idle' as string,
  setStatus: vi.fn(),
  isStopping: false,
  isProcessing: false,
  isSaving: false,
};

vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => mockRecordingState,
}));

// Mock Analytics
const mockTrackButtonClick = vi.fn();
vi.mock('@/lib/analytics', () => ({
  default: {
    trackButtonClick: (...args: unknown[]) => mockTrackButtonClick(...args),
  },
}));

// ── Helpers ───────────────────────────────────────────────────────────

/**
 * Emits a simulated Tauri event to the LAST registered listener matching
 * the given event name. The hook re-registers listeners when dependencies
 * change, so the last one has the freshest closure.
 */
function emitTauriEvent(eventName: string, payload?: unknown) {
  const matching = registeredListeners.filter((l) => l.event === eventName);
  if (matching.length > 0) {
    matching[matching.length - 1].callback({ payload });
  }
}

/**
 * Sets the mock recording state to simulate an active recording session.
 */
function setRecordingActive() {
  mockRecordingState.isRecording = true;
  mockRecordingState.isActive = true;
  mockRecordingState.status = 'recording';
}

/**
 * Resets the mock recording state to idle.
 */
function setRecordingIdle() {
  mockRecordingState.isRecording = false;
  mockRecordingState.isActive = false;
  mockRecordingState.status = 'idle';
}

/**
 * Flushes all pending microtasks/promises so that the async
 * setupListeners() inside useEffect completes and listeners
 * are registered.
 */
async function flushAsyncListenerSetup() {
  await act(async () => {
    // Flush microtask queue so the async listen() calls resolve
    await Promise.resolve();
    await Promise.resolve();
  });
}

/**
 * Helper to set up the hook in recording-active state, detect a meeting app,
 * wait past debounce, and fire system-audio-stopped to show the dialog.
 * Returns the renderHook result.
 */
async function setupAndShowDialog(onStop: () => void) {
  setRecordingActive();

  const hookResult = renderHook(() => useCallEndDetection(onStop));

  // Wait for async listener setup
  await flushAsyncListenerSetup();

  // 1. Detect a meeting app
  act(() => {
    emitTauriEvent('system-audio-started', ['Zoom']);
  });

  // The hook re-renders because lastDetectedApps changed,
  // which triggers useEffect cleanup + re-setup.
  // We need to flush the async setup again.
  await flushAsyncListenerSetup();

  // 2. Advance time past the 3-second debounce
  act(() => {
    vi.advanceTimersByTime(4000);
  });

  // 3. Fire system-audio-stopped
  act(() => {
    emitTauriEvent('system-audio-stopped');
  });

  return hookResult;
}

// ── Test Suite ────────────────────────────────────────────────────────

describe('useCallEndDetection', () => {
  const mockOnStopRecording = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    registeredListeners = [];
    setRecordingIdle();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ── Initial State ─────────────────────────────────────────────────

  /**
   * Verifies the hook returns the correct initial state with the dialog
   * hidden, no detected apps, and handler functions defined.
   */
  it('should return initial state with dialog hidden and empty detected apps', () => {
    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    expect(result.current.showCallEndedDialog).toBe(false);
    expect(result.current.lastDetectedApps).toEqual([]);
    expect(typeof result.current.handleStopFromCallEnd).toBe('function');
    expect(typeof result.current.handleContinueFromCallEnd).toBe('function');
  });

  // ── Event Listener Setup ──────────────────────────────────────────

  /**
   * Verifies that the hook registers listeners for both
   * 'system-audio-started' and 'system-audio-stopped' Tauri events
   * after the async setup completes.
   */
  it('should register listeners for system-audio-started and system-audio-stopped events', async () => {
    renderHook(() => useCallEndDetection(mockOnStopRecording));

    // Wait for async setupListeners to complete
    await flushAsyncListenerSetup();

    const eventNames = registeredListeners.map((l) => l.event);
    expect(eventNames).toContain('system-audio-started');
    expect(eventNames).toContain('system-audio-stopped');
  });

  // ── system-audio-started Tracking ─────────────────────────────────

  /**
   * Verifies that when a 'system-audio-started' event fires with app names,
   * the hook updates lastDetectedApps accordingly.
   */
  it('should update lastDetectedApps when system-audio-started fires', async () => {
    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    await flushAsyncListenerSetup();

    act(() => {
      emitTauriEvent('system-audio-started', ['Zoom', 'Slack']);
    });

    expect(result.current.lastDetectedApps).toEqual(['Zoom', 'Slack']);
  });

  // ── Dialog Display Logic ──────────────────────────────────────────

  /**
   * Verifies the dialog is shown when all conditions are met:
   * recording is active, a meeting app was detected, debounce has passed,
   * and the dialog hasn't been dismissed.
   */
  it('should show dialog when recording is active, meeting app detected, and audio stops', async () => {
    const { result } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);
  });

  /**
   * Verifies the dialog is NOT shown when recording is not active,
   * even if a meeting app was detected and audio stops.
   */
  it('should NOT show dialog when recording is not active', async () => {
    setRecordingIdle();

    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    await flushAsyncListenerSetup();

    act(() => {
      emitTauriEvent('system-audio-started', ['Zoom']);
    });

    await flushAsyncListenerSetup();

    act(() => {
      vi.advanceTimersByTime(4000);
    });

    act(() => {
      emitTauriEvent('system-audio-stopped');
    });

    expect(result.current.showCallEndedDialog).toBe(false);
  });

  // ── Meeting App Detection Filtering ───────────────────────────────

  /**
   * Verifies the dialog is NOT shown when the detected apps are not
   * recognized meeting/communication apps (e.g., Spotify).
   */
  it('should NOT show dialog when detected apps are not meeting apps', async () => {
    setRecordingActive();

    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    await flushAsyncListenerSetup();

    act(() => {
      emitTauriEvent('system-audio-started', ['Spotify', 'VLC Media Player']);
    });

    await flushAsyncListenerSetup();

    act(() => {
      vi.advanceTimersByTime(4000);
    });

    act(() => {
      emitTauriEvent('system-audio-stopped');
    });

    expect(result.current.showCallEndedDialog).toBe(false);
  });

  /**
   * Verifies the dialog IS shown for various known meeting apps
   * including Teams, Google Meet (via Chrome), Discord, etc.
   * Uses parameterized tests to avoid redundant test bodies.
   */
  it.each([
    ['Zoom'],
    ['Microsoft Teams'],
    ['Google Chrome'],
    ['Slack'],
    ['Discord'],
    ['Webex'],
    ['Skype'],
    ['FaceTime'],
    ['Brave Browser'],
    ['Firefox'],
    ['Safari'],
    ['Arc'],
    ['Microsoft Edge'],
  ])('should show dialog for known meeting app: %s', async (appName) => {
    setRecordingActive();

    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    await flushAsyncListenerSetup();

    act(() => {
      emitTauriEvent('system-audio-started', [appName]);
    });

    await flushAsyncListenerSetup();

    act(() => {
      vi.advanceTimersByTime(4000);
    });

    act(() => {
      emitTauriEvent('system-audio-stopped');
    });

    expect(result.current.showCallEndedDialog).toBe(true);
  });

  // ── Debounce Logic ────────────────────────────────────────────────

  /**
   * Verifies the debounce works: after the first dialog is shown and
   * dismissed, the dialog can be dismissed correctly.
   */
  it('should allow dismissing dialog after it is shown', async () => {
    const { result } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);

    // Dismiss the dialog
    act(() => {
      result.current.handleContinueFromCallEnd();
    });

    expect(result.current.showCallEndedDialog).toBe(false);
  });

  // ── handleStopFromCallEnd ─────────────────────────────────────────

  /**
   * Verifies that calling handleStopFromCallEnd hides the dialog
   * and invokes the onStopRecording callback.
   */
  it('should hide dialog and call onStopRecording when handleStopFromCallEnd is called', async () => {
    const { result } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);

    // User chooses to stop
    act(() => {
      result.current.handleStopFromCallEnd();
    });

    expect(result.current.showCallEndedDialog).toBe(false);
    expect(mockOnStopRecording).toHaveBeenCalledTimes(1);
  });

  // ── handleContinueFromCallEnd ─────────────────────────────────────

  /**
   * Verifies that calling handleContinueFromCallEnd hides the dialog
   * without calling onStopRecording.
   */
  it('should hide dialog without stopping recording when handleContinueFromCallEnd is called', async () => {
    const { result } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);

    // User chooses to continue
    act(() => {
      result.current.handleContinueFromCallEnd();
    });

    expect(result.current.showCallEndedDialog).toBe(false);
    expect(mockOnStopRecording).not.toHaveBeenCalled();
  });

  // ── Dismiss Persistence Within Recording Session ──────────────────

  /**
   * Verifies that once the user dismisses the dialog via "Continue Recording",
   * subsequent system-audio-stopped events do NOT re-show the dialog
   * within the same recording session.
   */
  it('should NOT re-show dialog after user dismisses it within the same recording session', async () => {
    const { result } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);

    act(() => {
      result.current.handleContinueFromCallEnd();
    });

    expect(result.current.showCallEndedDialog).toBe(false);

    // Wait for re-registration of listeners after state change
    await flushAsyncListenerSetup();

    // Another audio stop event - should remain dismissed
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    act(() => {
      emitTauriEvent('system-audio-stopped');
    });

    expect(result.current.showCallEndedDialog).toBe(false);
  });

  // ── Auto-dismiss When Recording Stops ─────────────────────────────

  /**
   * Verifies that the dialog auto-dismisses when recording stops
   * from another source (e.g., user clicks stop in main UI).
   */
  it('should auto-dismiss dialog when recording stops from another source', async () => {
    const { result, rerender } = await setupAndShowDialog(mockOnStopRecording);

    expect(result.current.showCallEndedDialog).toBe(true);

    // Simulate recording stopping from another source
    setRecordingIdle();

    // Re-render to trigger the useEffect that watches isRecording
    rerender();

    expect(result.current.showCallEndedDialog).toBe(false);
  });

  // ── Analytics Tracking ────────────────────────────────────────────

  /**
   * Verifies that analytics is tracked when the call-ended dialog
   * is triggered by a system-audio-stopped event.
   */
  it('should track analytics event when call-ended dialog is shown', async () => {
    await setupAndShowDialog(mockOnStopRecording);

    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'call_ended_detected',
      'system_audio'
    );
  });

  // ── Cleanup ───────────────────────────────────────────────────────

  /**
   * Verifies that event listeners are cleaned up when the hook unmounts,
   * preventing memory leaks.
   */
  it('should clean up event listeners on unmount', async () => {
    const { unmount } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    // Wait for async setup to complete
    await flushAsyncListenerSetup();

    const listenerCountBefore = registeredListeners.length;
    expect(listenerCountBefore).toBeGreaterThan(0);

    unmount();

    expect(registeredListeners.length).toBe(0);
  });

  // ── No Detected Apps ──────────────────────────────────────────────

  /**
   * Verifies the dialog is NOT shown when system-audio-stopped fires
   * but no apps were ever detected via system-audio-started.
   */
  it('should NOT show dialog when no apps have been detected', async () => {
    setRecordingActive();

    const { result } = renderHook(() =>
      useCallEndDetection(mockOnStopRecording)
    );

    await flushAsyncListenerSetup();

    act(() => {
      vi.advanceTimersByTime(4000);
    });

    act(() => {
      emitTauriEvent('system-audio-stopped');
    });

    expect(result.current.showCallEndedDialog).toBe(false);
  });
});
