/**
 * Unit tests for MeetingDetectedToast component
 *
 * Tests cover:
 * - Toast is shown when system-audio-started fires with a known meeting app
 * - Toast is NOT shown during the startup grace period (first 15 s)
 * - Toast is dismissed when system-audio-stopped fires
 * - Recording start button triggers recording when on home page (dispatches custom event)
 * - Recording start button navigates to home page when on another route
 * - Dismiss button hides the toast and sets the dismissed flag
 * - Always-on-top behavior when meeting is detected
 * - Always-on-top is removed when toast is dismissed or recording starts
 * - Dismissed flag prevents re-showing toast for the same session
 * - New system-audio-started event resets dismissed flag
 * - Component renders nothing (null) – only manages side-effects
 */

import React from 'react';

// ── Mocks ────────────────────────────────────────────────────────────────────

const mockRouterPush = jest.fn();
let mockPathname = '/';

const mockSetAlwaysOnTop = jest.fn().mockResolvedValue(undefined);
const mockSetFocus = jest.fn().mockResolvedValue(undefined);
const mockGetCurrentWindow = jest.fn().mockReturnValue({
  setAlwaysOnTop: mockSetAlwaysOnTop,
  setFocus: mockSetFocus,
});

// Tauri event listener registry
type ListenerFn = (event: { payload: unknown }) => void;
const registeredListeners: Record<string, ListenerFn[]> = {};
const mockUnlisten = jest.fn();

const mockListen = jest.fn().mockImplementation(
  (eventName: string, handler: ListenerFn) => {
    if (!registeredListeners[eventName]) {
      registeredListeners[eventName] = [];
    }
    registeredListeners[eventName].push(handler);
    return Promise.resolve(mockUnlisten);
  }
);

/** Helper: fire a Tauri event to all registered listeners */
function emitTauriEvent(eventName: string, payload?: unknown) {
  const handlers = registeredListeners[eventName] ?? [];
  handlers.forEach((h) => h({ payload }));
}

jest.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

jest.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mockGetCurrentWindow(),
}));

jest.mock('next/navigation', () => ({
  useRouter: () => ({ push: mockRouterPush }),
  usePathname: () => mockPathname,
}));

const mockToast = Object.assign(jest.fn(), {
  success: jest.fn(),
  error: jest.fn(),
  dismiss: jest.fn(),
  info: jest.fn(),
});

jest.mock('sonner', () => ({
  toast: mockToast,
}));

jest.mock('@/lib/analytics', () => ({
  default: {
    trackButtonClick: jest.fn(),
    track: jest.fn(),
  },
}));

// ── Imports (after mocks) ────────────────────────────────────────────────────

import { render, fireEvent, waitFor, act } from '@testing-library/react';
import { MeetingDetectedToast } from '../MeetingDetectedToast';

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Flush all pending microtasks so async listen() calls resolve */
async function flushAsyncSetup() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/**
 * Render the component, flush async setup, then advance timers past the
 * 15 s startup grace period so detection events are accepted.
 */
async function renderAndWaitReady() {
  jest.useFakeTimers();
  const result = render(<MeetingDetectedToast />);
  await flushAsyncSetup();
  // Advance past the 15-second isReady guard
  act(() => {
    jest.advanceTimersByTime(16_000);
  });
  return result;
}

// ── Test Suite ───────────────────────────────────────────────────────────────

describe('MeetingDetectedToast', () => {
  let dispatchEventSpy: jest.SpyInstance;
  let sessionStorageSpy: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    // Reset listener registry
    Object.keys(registeredListeners).forEach((k) => delete registeredListeners[k]);
    mockPathname = '/';
    mockUnlisten.mockReset();
  });

  afterEach(() => {
    jest.useRealTimers();
    dispatchEventSpy?.mockRestore();
    sessionStorageSpy?.mockRestore();
  });

  // ── Event Listener Registration ──────────────────────────────────────────

  describe('Event Listener Setup', () => {
    it('should register system-audio-started and system-audio-stopped listeners on mount', async () => {
      render(<MeetingDetectedToast />);
      await flushAsyncSetup();

      expect(mockListen).toHaveBeenCalledWith(
        'system-audio-started',
        expect.any(Function)
      );
      expect(mockListen).toHaveBeenCalledWith(
        'system-audio-stopped',
        expect.any(Function)
      );
    });

    it('should clean up event listeners on unmount', async () => {
      const { unmount } = render(<MeetingDetectedToast />);
      await flushAsyncSetup();

      unmount();

      // unlisten should have been called for both listeners
      expect(mockUnlisten).toHaveBeenCalledTimes(2);
    });
  });

  // ── Grace Period ─────────────────────────────────────────────────────────

  describe('Startup Grace Period', () => {
    it('should NOT show toast during the 15-second startup grace period', async () => {
      jest.useFakeTimers();
      render(<MeetingDetectedToast />);
      await flushAsyncSetup();

      // Fire detection BEFORE grace period ends
      act(() => {
        emitTauriEvent('system-audio-started', ['Microsoft Teams']);
      });

      // Toast should NOT be shown yet
      expect(mockToast).not.toHaveBeenCalled();
    });

    it('should show toast AFTER the 15-second startup grace period', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => {
        expect(mockToast).toHaveBeenCalledWith(
          expect.anything(),
          expect.objectContaining({
            id: 'meeting-detected',
            duration: Infinity,
            position: 'top-center',
          })
        );
      });
    });
  });

  // ── Toast Display ────────────────────────────────────────────────────────

  describe('Toast Display', () => {
    it('should show toast when system-audio-started fires with a meeting app', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Microsoft Teams']);
      });

      await waitFor(() => {
        expect(mockToast).toHaveBeenCalledWith(
          expect.anything(),
          expect.objectContaining({ id: 'meeting-detected', duration: Infinity })
        );
      });
    });

    it('should include detected app name in toast message', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => {
        const toastContent = mockToast.mock.calls[0]?.[0];
        const { container } = render(toastContent);
        expect(container.textContent).toContain('Zoom');
      });
    });

    it('should dismiss toast when system-audio-stopped fires', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      act(() => {
        emitTauriEvent('system-audio-stopped');
      });

      await waitFor(() => {
        expect(mockToast.dismiss).toHaveBeenCalledWith('meeting-detected');
      });
    });

    it('should NOT use useMicActivityMonitor (no mic-activity-detected dependency)', async () => {
      // The component should only use Tauri event listeners, not the mic monitor hook.
      // Verify no mic-activity-detected listener is registered.
      render(<MeetingDetectedToast />);
      await flushAsyncSetup();

      const listenedEvents = mockListen.mock.calls.map((c) => c[0]);
      expect(listenedEvents).not.toContain('mic-activity-detected');
      expect(listenedEvents).not.toContain('mic-activity-stopped');
    });
  });

  // ── Start Recording – Home Page ──────────────────────────────────────────

  describe('Start Recording – Home Page', () => {
    beforeEach(() => {
      sessionStorageSpy = jest.spyOn(Storage.prototype, 'setItem');
      dispatchEventSpy = jest.spyOn(window, 'dispatchEvent');
    });

    it('should dispatch start-recording-from-sidebar event when on home page', async () => {
      mockPathname = '/';
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Microsoft Teams']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const startButton = container.querySelector('button:last-child');

      expect(startButton?.textContent).toContain('Start Recording');
      fireEvent.click(startButton!);

      expect(sessionStorageSpy).toHaveBeenCalledWith('autoStartRecording', 'true');
      expect(dispatchEventSpy).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'start-recording-from-sidebar' })
      );
      expect(mockRouterPush).not.toHaveBeenCalled();
    });

    it('should dismiss toast and remove always-on-top when Start Recording is clicked', async () => {
      mockPathname = '/';
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const startButton = container.querySelector('button:last-child');

      fireEvent.click(startButton!);

      expect(mockToast.dismiss).toHaveBeenCalledWith('meeting-detected');

      await waitFor(() => {
        expect(mockSetAlwaysOnTop).toHaveBeenCalledWith(false);
      });
    });
  });

  // ── Start Recording – Other Page ─────────────────────────────────────────

  describe('Start Recording – Other Page', () => {
    beforeEach(() => {
      sessionStorageSpy = jest.spyOn(Storage.prototype, 'setItem');
      dispatchEventSpy = jest.spyOn(window, 'dispatchEvent');
    });

    it('should navigate to home page when on a different route', async () => {
      mockPathname = '/meeting-details';
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const startButton = container.querySelector('button:last-child');

      fireEvent.click(startButton!);

      expect(sessionStorageSpy).toHaveBeenCalledWith('autoStartRecording', 'true');
      expect(mockRouterPush).toHaveBeenCalledWith('/');
      expect(dispatchEventSpy).not.toHaveBeenCalled();
    });
  });

  // ── Dismiss Button ───────────────────────────────────────────────────────

  describe('Dismiss Button', () => {
    it('should dismiss toast when Dismiss is clicked', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Slack']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const dismissButton = container.querySelector('button:first-child');

      expect(dismissButton?.textContent).toContain('Dismiss');
      fireEvent.click(dismissButton!);

      expect(mockToast.dismiss).toHaveBeenCalledWith('meeting-detected');
    });

    it('should remove always-on-top when Dismiss is clicked', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Slack']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      mockSetAlwaysOnTop.mockClear();

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const dismissButton = container.querySelector('button:first-child');

      fireEvent.click(dismissButton!);

      await waitFor(() => {
        expect(mockSetAlwaysOnTop).toHaveBeenCalledWith(false);
      });
    });

    it('should NOT re-show toast immediately after dismiss (before next audio-started event)', async () => {
      await renderAndWaitReady();

      // First detection
      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => expect(mockToast).toHaveBeenCalled());

      const toastContent = mockToast.mock.calls[0][0];
      const { container } = render(toastContent);
      const dismissButton = container.querySelector('button:first-child');
      fireEvent.click(dismissButton!);

      mockToast.mockClear();

      // system-audio-stopped fires — should NOT re-show the toast
      // (dismissedRef is true, meetingDetected becomes false)
      act(() => {
        emitTauriEvent('system-audio-stopped');
      });

      // Toast should NOT be called again — only toast.dismiss should have been called
      expect(mockToast).not.toHaveBeenCalled();
    });
  });

  // ── Always-on-Top Behavior ───────────────────────────────────────────────

  describe('Always-on-Top Behavior', () => {
    it('should set window always-on-top when meeting is detected', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Microsoft Teams']);
      });

      await waitFor(() => {
        expect(mockSetAlwaysOnTop).toHaveBeenCalledWith(true);
        expect(mockSetFocus).toHaveBeenCalled();
      });
    });

    it('should remove always-on-top when meeting is no longer detected', async () => {
      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => expect(mockSetAlwaysOnTop).toHaveBeenCalledWith(true));
      mockSetAlwaysOnTop.mockClear();

      act(() => {
        emitTauriEvent('system-audio-stopped');
      });

      await waitFor(() => {
        expect(mockSetAlwaysOnTop).toHaveBeenCalledWith(false);
      });
    });

    it('should handle errors gracefully when setting always-on-top fails', async () => {
      // Make ALL setAlwaysOnTop calls fail (both the initial false call and the true call)
      mockSetAlwaysOnTop.mockRejectedValue(new Error('Permission denied'));
      const consoleSpy = jest.spyOn(console, 'error').mockImplementation();

      await renderAndWaitReady();

      act(() => {
        emitTauriEvent('system-audio-started', ['Zoom']);
      });

      await waitFor(() => {
        // The error message could be either 'Failed to set always-on-top:' or
        // 'Failed to remove always-on-top:' depending on which call fails first.
        // Both are valid graceful error handling.
        const calls = consoleSpy.mock.calls;
        const hasAlwaysOnTopError = calls.some(
          (call) =>
            (call[0] === 'Failed to set always-on-top:' ||
              call[0] === 'Failed to remove always-on-top:') &&
            call[1] instanceof Error
        );
        expect(hasAlwaysOnTopError).toBe(true);
      });

      consoleSpy.mockRestore();
      mockSetAlwaysOnTop.mockReset().mockResolvedValue(undefined);
    });
  });

  // ── Component Rendering ──────────────────────────────────────────────────

  describe('Component Rendering', () => {
    it('should render nothing (null) – only manages side-effects', () => {
      const { container } = render(<MeetingDetectedToast />);
      expect(container.innerHTML).toBe('');
    });
  });
});
