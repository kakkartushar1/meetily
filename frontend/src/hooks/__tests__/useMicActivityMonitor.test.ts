/**
 * Unit tests for useMicActivityMonitor hook
 *
 * Tests cover:
 * - Initial state loading from Tauri backend
 * - Event listener setup and cleanup
 * - Preference toggling
 * - Detection dismissal
 * - State transitions on events
 */

// Mock Tauri APIs
const mockInvoke = jest.fn();
const mockListen = jest.fn();
const mockUnlisten = jest.fn();

jest.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

jest.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

jest.mock('sonner', () => ({
  toast: Object.assign(jest.fn(), {
    success: jest.fn(),
    error: jest.fn(),
    dismiss: jest.fn(),
  }),
}));

import { renderHook, act } from '@testing-library/react';
import { useMicActivityMonitor } from '../useMicActivityMonitor';

describe('useMicActivityMonitor', () => {
  beforeEach(() => {
    jest.clearAllMocks();

    // Default mock implementations
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'get_mic_activity_monitoring_status':
          return Promise.resolve(false);
        case 'get_mic_activity_monitoring_preference':
          return Promise.resolve(false);
        case 'set_mic_activity_monitoring_preference':
          return Promise.resolve();
        case 'dismiss_mic_activity_detection':
          return Promise.resolve();
        default:
          return Promise.resolve();
      }
    });

    // Mock listen to return an unlisten function
    mockListen.mockResolvedValue(mockUnlisten);
  });

  it('should load initial state from backend', async () => {
    const { result } = renderHook(() => useMicActivityMonitor());

    // Wait for async effects
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(mockInvoke).toHaveBeenCalledWith('get_mic_activity_monitoring_status');
    expect(mockInvoke).toHaveBeenCalledWith('get_mic_activity_monitoring_preference');
    expect(result.current.isMonitoring).toBe(false);
    expect(result.current.preferenceEnabled).toBe(false);
    expect(result.current.meetingDetected).toBe(false);
  });

  it('should reflect enabled state from backend', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'get_mic_activity_monitoring_status':
          return Promise.resolve(true);
        case 'get_mic_activity_monitoring_preference':
          return Promise.resolve(true);
        default:
          return Promise.resolve();
      }
    });

    const { result } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(result.current.isMonitoring).toBe(true);
    expect(result.current.preferenceEnabled).toBe(true);
  });

  it('should set up event listeners on mount', async () => {
    renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    expect(mockListen).toHaveBeenCalledWith(
      'mic-activity-detected',
      expect.any(Function)
    );
    expect(mockListen).toHaveBeenCalledWith(
      'mic-activity-stopped',
      expect.any(Function)
    );
  });

  it('should clean up event listeners on unmount', async () => {
    const { unmount } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    unmount();

    // The unlisten functions should have been called
    expect(mockUnlisten).toHaveBeenCalled();
  });

  it('should toggle preference via backend command', async () => {
    const { result } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    await act(async () => {
      await result.current.setPreference(true);
    });

    expect(mockInvoke).toHaveBeenCalledWith(
      'set_mic_activity_monitoring_preference',
      { enabled: true }
    );
    expect(result.current.preferenceEnabled).toBe(true);
    expect(result.current.isMonitoring).toBe(true);
  });

  it('should dismiss detection via backend command', async () => {
    const { result } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    await act(async () => {
      await result.current.dismissDetection();
    });

    expect(mockInvoke).toHaveBeenCalledWith('dismiss_mic_activity_detection');
    expect(result.current.meetingDetected).toBe(false);
  });

  it('should handle backend errors gracefully when loading state', async () => {
    mockInvoke.mockRejectedValue(new Error('Backend unavailable'));

    const consoleSpy = jest.spyOn(console, 'error').mockImplementation();

    const { result } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    // Should not crash, defaults to false
    expect(result.current.isMonitoring).toBe(false);
    expect(result.current.preferenceEnabled).toBe(false);

    consoleSpy.mockRestore();
  });

  it('should reset meetingDetected when disabling preference', async () => {
    const { result } = renderHook(() => useMicActivityMonitor());

    await act(async () => {
      await new Promise((r) => setTimeout(r, 50));
    });

    // Enable first
    await act(async () => {
      await result.current.setPreference(true);
    });

    // Disable
    await act(async () => {
      await result.current.setPreference(false);
    });

    expect(result.current.meetingDetected).toBe(false);
    expect(result.current.isMonitoring).toBe(false);
    expect(result.current.preferenceEnabled).toBe(false);
  });
});
