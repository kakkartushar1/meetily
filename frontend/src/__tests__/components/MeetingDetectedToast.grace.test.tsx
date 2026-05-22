/**
 * Unit tests for MeetingDetectedToast component — startup grace period
 *
 * These tests verify that the MeetingDetectedToast component does NOT show
 * the toast notification or bring the window to the front during the first
 * 10 seconds after mount (the "isReady" grace period). This prevents false
 * positive "Meeting Detected" popups on app startup.
 *
 * Tests cover:
 * - Toast is NOT shown during the 10s grace period even if meetingDetected=true
 * - Always-on-top is NOT set during the grace period
 * - Toast IS shown after the grace period ends
 * - Toast is properly dismissed when meetingDetected becomes false
 */

import React from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor } from '@testing-library/react';

// --- Mocks ---
const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockUnlisten = vi.fn();
const mockRouterPush = vi.fn();
let mockPathname = '/';

const mockSetAlwaysOnTop = vi.fn().mockResolvedValue(undefined);
const mockSetFocus = vi.fn().mockResolvedValue(undefined);
const mockGetCurrentWindow = vi.fn().mockReturnValue({
  setAlwaysOnTop: mockSetAlwaysOnTop,
  setFocus: mockSetFocus,
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mockGetCurrentWindow(),
}));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: mockRouterPush }),
  usePathname: () => mockPathname,
}));

const mockToast = Object.assign(vi.fn(), {
  success: vi.fn(),
  error: vi.fn(),
  dismiss: vi.fn(),
  info: vi.fn(),
});

vi.mock('sonner', () => ({
  toast: mockToast,
}));

// Helper to simulate mic activity monitor state
let mockMeetingDetected = false;
let mockIsMonitoring = true;
const mockDismissDetection = vi.fn().mockResolvedValue(undefined);
let mockDeviceName = 'Test Microphone';

vi.mock('@/hooks/useMicActivityMonitor', () => ({
  useMicActivityMonitor: () => ({
    meetingDetected: mockMeetingDetected,
    isMonitoring: mockIsMonitoring,
    dismissDetection: mockDismissDetection,
    deviceName: mockDeviceName,
    preferenceEnabled: true,
    setPreference: vi.fn(),
  }),
}));

// Dynamic import after mocks are set up
const { MeetingDetectedToast } = await import('@/components/MeetingDetectedToast');

describe('MeetingDetectedToast — startup grace period', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    mockMeetingDetected = false;
    mockIsMonitoring = true;
    mockPathname = '/';
    mockDeviceName = 'Test Microphone';

    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'get_mic_activity_monitoring_status':
          return Promise.resolve(true);
        case 'get_mic_activity_monitoring_preference':
          return Promise.resolve(true);
        case 'dismiss_mic_activity_detection':
          return Promise.resolve();
        default:
          return Promise.resolve();
      }
    });

    mockListen.mockResolvedValue(mockUnlisten);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('should NOT show toast during the 10s grace period even if meetingDetected is true', () => {
    mockMeetingDetected = true;
    mockIsMonitoring = true;

    render(<MeetingDetectedToast />);

    // During grace period (isReady=false), the toast should NOT be shown
    // Instead, it should dismiss any existing toast
    expect(mockToast).not.toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        id: 'meeting-detected',
        duration: Infinity,
      })
    );
  });

  it('should NOT set always-on-top during the grace period', () => {
    mockMeetingDetected = true;
    mockIsMonitoring = true;

    render(<MeetingDetectedToast />);

    // During grace period, always-on-top should NOT be set to true
    // The component guards with `isReady` before calling setAlwaysOnTop(true)
    const trueCallCount = mockSetAlwaysOnTop.mock.calls.filter(
      (call: unknown[]) => call[0] === true
    ).length;
    expect(trueCallCount).toBe(0);
  });

  it('should show toast AFTER the 10s grace period ends', () => {
    mockMeetingDetected = true;
    mockIsMonitoring = true;

    const { rerender } = render(<MeetingDetectedToast />);

    // Toast should NOT have been shown yet (grace period active)
    expect(mockToast).not.toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        id: 'meeting-detected',
        duration: Infinity,
      })
    );

    // Advance past the 10s grace period
    vi.advanceTimersByTime(11000);

    // Re-render to pick up the isReady state change
    rerender(<MeetingDetectedToast />);

    // Now the toast should be shown
    expect(mockToast).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        id: 'meeting-detected',
        duration: Infinity,
        position: 'top-center',
      })
    );
  });

  it('should dismiss toast when meetingDetected becomes false after grace period', async () => {
    mockMeetingDetected = true;
    mockIsMonitoring = true;

    const { rerender } = render(<MeetingDetectedToast />);

    // Advance past grace period
    vi.advanceTimersByTime(11000);
    rerender(<MeetingDetectedToast />);

    // Now set meetingDetected to false
    mockMeetingDetected = false;
    rerender(<MeetingDetectedToast />);

    expect(mockToast.dismiss).toHaveBeenCalledWith('meeting-detected');
  });

  it('should render nothing (null) during and after grace period', () => {
    mockMeetingDetected = true;
    mockIsMonitoring = true;

    const { container } = render(<MeetingDetectedToast />);
    expect(container.innerHTML).toBe('');

    // Advance past grace period
    vi.advanceTimersByTime(11000);
    expect(container.innerHTML).toBe('');
  });
});
