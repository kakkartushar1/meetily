import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CallEndedDialog } from '@/components/CallEndedDialog';

// ── Mocks ────────────────────────────────────────────────────────────────────

// Mock Analytics module
const mockTrackButtonClick = vi.fn();
vi.mock('@/lib/analytics', () => ({
  default: {
    trackButtonClick: (...args: unknown[]) => mockTrackButtonClick(...args),
  },
}));

// ── Test Suite ───────────────────────────────────────────────────────────────

describe('CallEndedDialog', () => {
  const defaultProps = {
    isOpen: true,
    onStopRecording: vi.fn(),
    onContinueRecording: vi.fn(),
    lastDetectedApps: undefined as string[] | undefined,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ── Rendering Tests ──────────────────────────────────────────────────────

  /**
   * Verifies the dialog renders its title, description, and both action
   * buttons when the isOpen prop is true.
   */
  it('should render dialog content when isOpen is true', () => {
    render(<CallEndedDialog {...defaultProps} />);

    expect(screen.getByText('Meeting Call Ended')).toBeInTheDocument();
    expect(screen.getByText(/Would you like to stop recording/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /stop recording and save meeting/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /continue recording/i })).toBeInTheDocument();
  });

  /**
   * Verifies the dialog does not render any visible content when
   * the isOpen prop is false.
   */
  it('should not render dialog content when isOpen is false', () => {
    render(<CallEndedDialog {...defaultProps} isOpen={false} />);

    expect(screen.queryByText('Meeting Call Ended')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /stop recording and save meeting/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /continue recording/i })).not.toBeInTheDocument();
  });

  // ── Dynamic App Name Display ─────────────────────────────────────────────

  /**
   * Verifies that when lastDetectedApps contains a single app name,
   * the dialog displays a message mentioning that specific app.
   */
  it('should display detected app name in the description when a single app is provided', () => {
    render(
      <CallEndedDialog {...defaultProps} lastDetectedApps={['Zoom']} />
    );

    expect(screen.getByText(/It looks like Zoom has ended\./)).toBeInTheDocument();
  });

  /**
   * Verifies that when lastDetectedApps contains multiple app names,
   * the dialog displays all of them joined by commas.
   */
  it('should display multiple detected app names joined by comma', () => {
    render(
      <CallEndedDialog
        {...defaultProps}
        lastDetectedApps={['Zoom', 'Microsoft Teams']}
      />
    );

    expect(
      screen.getByText(/It looks like Zoom, Microsoft Teams has ended\./)
    ).toBeInTheDocument();
  });

  /**
   * Verifies that when lastDetectedApps is undefined, the dialog
   * shows a generic fallback message.
   */
  it('should display generic message when lastDetectedApps is undefined', () => {
    render(<CallEndedDialog {...defaultProps} lastDetectedApps={undefined} />);

    expect(
      screen.getByText(/It looks like your meeting call has ended\./)
    ).toBeInTheDocument();
  });

  /**
   * Verifies that when lastDetectedApps is an empty array, the dialog
   * shows a generic fallback message.
   */
  it('should display generic message when lastDetectedApps is an empty array', () => {
    render(<CallEndedDialog {...defaultProps} lastDetectedApps={[]} />);

    expect(
      screen.getByText(/It looks like your meeting call has ended\./)
    ).toBeInTheDocument();
  });

  // ── Stop Recording Button ────────────────────────────────────────────────

  /**
   * Verifies clicking the "Stop & Save" button triggers the onStopRecording
   * callback and tracks the analytics event.
   */
  it('should call onStopRecording and track analytics when Stop & Save is clicked', async () => {
    const user = userEvent.setup();
    render(<CallEndedDialog {...defaultProps} />);

    const stopButton = screen.getByRole('button', { name: /stop recording and save meeting/i });
    await user.click(stopButton);

    expect(defaultProps.onStopRecording).toHaveBeenCalledTimes(1);
    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'call_ended_stop_recording',
      'call_ended_dialog'
    );
  });

  // ── Continue Recording Button ────────────────────────────────────────────

  /**
   * Verifies clicking the "Continue Recording" button triggers the
   * onContinueRecording callback and tracks the analytics event.
   */
  it('should call onContinueRecording and track analytics when Continue Recording is clicked', async () => {
    const user = userEvent.setup();
    render(<CallEndedDialog {...defaultProps} />);

    const continueButton = screen.getByRole('button', { name: /continue recording/i });
    await user.click(continueButton);

    expect(defaultProps.onContinueRecording).toHaveBeenCalledTimes(1);
    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'call_ended_continue_recording',
      'call_ended_dialog'
    );
  });

  // ── Analytics Tracking ───────────────────────────────────────────────────

  /**
   * Verifies that analytics is NOT called before any button interaction,
   * ensuring tracking only fires on explicit user action.
   */
  it('should not track analytics on initial render', () => {
    render(<CallEndedDialog {...defaultProps} />);

    expect(mockTrackButtonClick).not.toHaveBeenCalled();
  });

  /**
   * Verifies that clicking Stop and then Continue (in separate renders)
   * tracks distinct analytics events with correct identifiers.
   */
  it('should track different analytics events for stop vs continue', async () => {
    const user = userEvent.setup();
    const { unmount } = render(<CallEndedDialog {...defaultProps} />);

    await user.click(screen.getByRole('button', { name: /stop recording and save meeting/i }));
    expect(mockTrackButtonClick).toHaveBeenLastCalledWith(
      'call_ended_stop_recording',
      'call_ended_dialog'
    );

    unmount();
    vi.clearAllMocks();

    render(<CallEndedDialog {...defaultProps} />);
    await user.click(screen.getByRole('button', { name: /continue recording/i }));
    expect(mockTrackButtonClick).toHaveBeenLastCalledWith(
      'call_ended_continue_recording',
      'call_ended_dialog'
    );
  });

  // ── Button Content ───────────────────────────────────────────────────────

  /**
   * Verifies the "Stop & Save" button contains the expected visible text.
   */
  it('should render Stop & Save button with correct text', () => {
    render(<CallEndedDialog {...defaultProps} />);

    expect(screen.getByText('Stop & Save')).toBeInTheDocument();
  });

  /**
   * Verifies the "Continue Recording" button contains the expected visible text.
   */
  it('should render Continue Recording button with correct text', () => {
    render(<CallEndedDialog {...defaultProps} />);

    expect(screen.getByText('Continue Recording')).toBeInTheDocument();
  });

  // ── Title ────────────────────────────────────────────────────────────────

  /**
   * Verifies the dialog title is rendered as "Meeting Call Ended".
   */
  it('should render the correct dialog title', () => {
    render(<CallEndedDialog {...defaultProps} />);

    expect(screen.getByText('Meeting Call Ended')).toBeInTheDocument();
  });
});
