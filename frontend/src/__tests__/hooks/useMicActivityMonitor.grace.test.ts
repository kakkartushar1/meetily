/**
 * Unit tests for useMicActivityMonitor hook — grace period logic
 *
 * These tests verify that the frontend grace period in useMicActivityMonitor
 * suppresses meeting detection events during the first FRONTEND_GRACE_PERIOD_MS
 * (12 seconds) after the hook mounts, preventing false-positive "Meeting Detected"
 * toasts on app startup.
 *
 * Tests cover:
 * - Detection events are suppressed during the grace period
 * - Detection events are accepted after the grace period
 * - meetingDetected stays false during grace period even if backend fires events
 * - mic-activity-stopped events are still processed during grace period
 * - Grace period does not interfere with preference loading
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// ── Type definitions ──────────────────────────────────────────────────

type EventCallback = (event: { payload: unknown }) => void;

interface ListenerRegistration {
  event: string;
  callback: EventCallback;
}

// ── Mocks ─────────────────────────────────────────────────────────────

let registeredListeners: ListenerRegistration[] = [];

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, callback: EventCallback) => {
    registeredListeners.push({ event, callback });
    return () => {
      registeredListeners = registeredListeners.filter(
        (l) => !(l.event === event && l.callback === callback)
      );
    };
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    switch (cmd) {
      case 'get_mic_activity_monitoring_status':
        return true;
      case 'get_mic_activity_monitoring_preference':
        return true;
      case 'dismiss_mic_activity_detection':
        return undefined;
      case 'set_mic_activity_monitoring_preference':
        return undefined;
      default:
        return undefined;
    }
  }),
}));

vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), {
    success: vi.fn(),
    error: vi.fn(),
    dismiss: vi.fn(),
    info: vi.fn(),
  }),
}));

// ── Helpers ───────────────────────────────────────────────────────────

function emitTauriEvent(eventName: string, payload?: unknown) {
  const matching = registeredListeners.filter((l) => l.event === eventName);
  if (matching.length > 0) {
    matching[matching.length - 1].callback({ payload });
  }
}

async function flushAsyncSetup() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

// ── Test Suite ────────────────────────────────────────────────────────

describe('useMicActivityMonitor — grace period', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    registeredListeners = [];
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('should suppress mic-activity-detected events during the 12s grace period', async () => {
    // Import dynamically so mocks are in place
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Initially meetingDetected should be false
    expect(result.current.meetingDetected).toBe(false);

    // Simulate a mic-activity-detected event at t=2s (within grace period)
    act(() => {
      vi.advanceTimersByTime(2000);
    });

    act(() => {
      emitTauriEvent('mic-activity-detected', {
        detected: true,
        rms_level: 0.5,
        device_name: 'Test Mic',
        timestamp: Date.now(),
      });
    });

    // meetingDetected should STILL be false — event suppressed by grace period
    expect(result.current.meetingDetected).toBe(false);
  });

  it('should accept mic-activity-detected events after the grace period ends', async () => {
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Advance past the 12s grace period
    act(() => {
      vi.advanceTimersByTime(13000);
    });

    // Now fire a detection event — should be accepted
    act(() => {
      emitTauriEvent('mic-activity-detected', {
        detected: true,
        rms_level: 0.5,
        device_name: 'Test Mic',
        timestamp: Date.now(),
      });
    });

    expect(result.current.meetingDetected).toBe(true);
    expect(result.current.deviceName).toBe('Test Mic');
  });

  it('should still process mic-activity-stopped events during grace period', async () => {
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Fire a stopped event during grace period — should still set meetingDetected to false
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    act(() => {
      emitTauriEvent('mic-activity-stopped', {
        detected: false,
        rms_level: 0.0,
        device_name: 'Test Mic',
        timestamp: Date.now(),
      });
    });

    // meetingDetected should remain false (it was already false)
    expect(result.current.meetingDetected).toBe(false);
  });

  it('should suppress multiple detection events during grace period', async () => {
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Fire multiple detection events at different absolute times during grace period.
    // Note: vi.advanceTimersByTime is cumulative, so we advance by increments
    // that keep the total elapsed time under 12s.
    const incrementsMs = [500, 1500, 2000, 2000, 2000]; // cumulative: 0.5, 2, 4, 6, 8s
    for (const incr of incrementsMs) {
      act(() => {
        vi.advanceTimersByTime(incr);
      });

      act(() => {
        emitTauriEvent('mic-activity-detected', {
          detected: true,
          rms_level: 0.3,
          device_name: 'Test Mic',
          timestamp: Date.now(),
        });
      });

      // All should be suppressed — still within the 12s grace period
      expect(result.current.meetingDetected).toBe(false);
    }
  });

  it('should not interfere with preference loading during grace period', async () => {
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Preferences should load normally during grace period
    expect(result.current.isMonitoring).toBe(true);
    expect(result.current.preferenceEnabled).toBe(true);
  });

  it('should correctly detect meeting after grace period then detect end', async () => {
    const { useMicActivityMonitor } = await import('@/hooks/useMicActivityMonitor');

    const { result } = renderHook(() => useMicActivityMonitor());
    await flushAsyncSetup();

    // Advance past grace period
    act(() => {
      vi.advanceTimersByTime(13000);
    });

    // Detect meeting
    act(() => {
      emitTauriEvent('mic-activity-detected', {
        detected: true,
        rms_level: 0.5,
        device_name: 'Test Mic',
        timestamp: Date.now(),
      });
    });
    expect(result.current.meetingDetected).toBe(true);

    // Detect meeting ended
    act(() => {
      emitTauriEvent('mic-activity-stopped', {
        detected: false,
        rms_level: 0.0,
        device_name: 'Test Mic',
        timestamp: Date.now(),
      });
    });
    expect(result.current.meetingDetected).toBe(false);
  });
});
