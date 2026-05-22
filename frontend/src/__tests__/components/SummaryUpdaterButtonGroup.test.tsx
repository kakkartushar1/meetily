import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SummaryUpdaterButtonGroup } from '@/components/MeetingDetails/SummaryUpdaterButtonGroup';

// ── Mocks ────────────────────────────────────────────────────────────────────

// Mock Analytics module
const mockTrackButtonClick = vi.fn();
vi.mock('@/lib/analytics', () => ({
  default: {
    trackButtonClick: (...args: unknown[]) => mockTrackButtonClick(...args),
  },
}));

// Mock lucide-react icons using React.createElement to avoid JSX parse issues
vi.mock('lucide-react', () => ({
  Copy: (props: any) => React.createElement('span', { 'data-testid': 'icon-copy', ...props }),
  Save: (props: any) => React.createElement('span', { 'data-testid': 'icon-save', ...props }),
  Loader2: (props: any) => React.createElement('span', { 'data-testid': 'icon-loader2', ...props }),
  RefreshCw: (props: any) => React.createElement('span', { 'data-testid': 'icon-refresh', ...props }),
}));

// ── Test Suite ───────────────────────────────────────────────────────────────

describe('SummaryUpdaterButtonGroup', () => {
  const defaultProps = {
    isSaving: false,
    isDirty: false,
    onSave: vi.fn().mockResolvedValue(undefined),
    onCopy: vi.fn().mockResolvedValue(undefined),
    onOpenFolder: vi.fn().mockResolvedValue(undefined),
    hasSummary: true,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ── Regenerate Button Rendering ──────────────────────────────────────────

  /**
   * Verifies that the Regenerate button renders when the
   * onRegenerateSummary prop is provided.
   */
  it('should render the Regenerate button when onRegenerateSummary prop is provided', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    expect(regenerateButton).toBeInTheDocument();
  });

  /**
   * Verifies that the Regenerate button does NOT render when the
   * onRegenerateSummary prop is not provided (undefined).
   */
  it('should NOT render the Regenerate button when onRegenerateSummary prop is not provided', () => {
    render(<SummaryUpdaterButtonGroup {...defaultProps} />);

    const regenerateButton = screen.queryByRole('button', { name: /regenerate the meeting summary/i });
    expect(regenerateButton).not.toBeInTheDocument();
  });

  // ── Regenerate Button Click ──────────────────────────────────────────────

  /**
   * Verifies that clicking the Regenerate button calls the
   * onRegenerateSummary callback.
   */
  it('should call onRegenerateSummary when the Regenerate button is clicked', async () => {
    const user = userEvent.setup();
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    await user.click(regenerateButton);

    expect(onRegenerateSummary).toHaveBeenCalledTimes(1);
  });

  // ── Loading State ────────────────────────────────────────────────────────

  /**
   * Verifies that the Regenerate button shows a loading state with
   * a Loader2 spinner icon and 'Regenerating...' text when
   * isRegenerating is true.
   */
  it('should show loading state with Loader2 spinner and Regenerating text when isRegenerating is true', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
        isRegenerating={true}
      />
    );

    // The Loader2 icon should be present inside the regenerate button
    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    const loaderIcon = regenerateButton.querySelector('[data-testid="icon-loader2"]');
    expect(loaderIcon).toBeInTheDocument();

    // The 'Regenerating...' text should be visible
    expect(screen.getByText('Regenerating...')).toBeInTheDocument();

    // The RefreshCw icon and 'Regenerate' text should NOT be present
    const refreshIcon = regenerateButton.querySelector('[data-testid="icon-refresh"]');
    expect(refreshIcon).not.toBeInTheDocument();
  });

  /**
   * Verifies that the Regenerate button shows the default state with
   * RefreshCw icon and 'Regenerate' text when isRegenerating is false.
   */
  it('should show default state with RefreshCw icon and Regenerate text when isRegenerating is false', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
        isRegenerating={false}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    const refreshIcon = regenerateButton.querySelector('[data-testid="icon-refresh"]');
    expect(refreshIcon).toBeInTheDocument();

    expect(screen.getByText('Regenerate')).toBeInTheDocument();
  });

  // ── Disabled States ──────────────────────────────────────────────────────

  /**
   * Verifies that the Regenerate button is disabled when
   * isRegenerating is true.
   */
  it('should disable the Regenerate button when isRegenerating is true', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
        isRegenerating={true}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    expect(regenerateButton).toBeDisabled();
  });

  /**
   * Verifies that the Regenerate button is disabled when
   * hasSummary is false, since there is no summary to regenerate.
   */
  it('should disable the Regenerate button when hasSummary is false', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        hasSummary={false}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    expect(regenerateButton).toBeDisabled();
  });

  /**
   * Verifies that the Regenerate button is enabled when both
   * hasSummary is true and isRegenerating is false.
   */
  it('should enable the Regenerate button when hasSummary is true and isRegenerating is false', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        hasSummary={true}
        onRegenerateSummary={onRegenerateSummary}
        isRegenerating={false}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    expect(regenerateButton).not.toBeDisabled();
  });

  // ── Analytics Tracking ───────────────────────────────────────────────────

  /**
   * Verifies that Analytics.trackButtonClick is called with
   * 'regenerate_summary' and 'meeting_details' when the
   * Regenerate button is clicked.
   */
  it('should call Analytics.trackButtonClick with correct args when Regenerate is clicked', async () => {
    const user = userEvent.setup();
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    await user.click(regenerateButton);

    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'regenerate_summary',
      'meeting_details'
    );
  });

  /**
   * Verifies that analytics is NOT called on initial render,
   * ensuring tracking only fires on explicit user action.
   */
  it('should not track analytics on initial render', () => {
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    expect(mockTrackButtonClick).not.toHaveBeenCalled();
  });

  /**
   * Verifies that clicking the Save button tracks the correct
   * analytics event distinct from the Regenerate button.
   */
  it('should track different analytics events for Save vs Regenerate', async () => {
    const user = userEvent.setup();
    const onRegenerateSummary = vi.fn().mockResolvedValue(undefined);
    render(
      <SummaryUpdaterButtonGroup
        {...defaultProps}
        onRegenerateSummary={onRegenerateSummary}
      />
    );

    // Click Save button
    const saveButton = screen.getByTitle('Save Changes');
    await user.click(saveButton);
    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'save_changes',
      'meeting_details'
    );

    vi.clearAllMocks();

    // Click Regenerate button
    const regenerateButton = screen.getByRole('button', { name: /regenerate the meeting summary/i });
    await user.click(regenerateButton);
    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'regenerate_summary',
      'meeting_details'
    );
  });

  // ── Copy Button Interaction ──────────────────────────────────────────────

  /**
   * Verifies that the Copy button tracks analytics with 'copy_summary'
   * and calls the onCopy callback.
   */
  it('should call onCopy and track analytics when Copy button is clicked', async () => {
    const user = userEvent.setup();
    render(<SummaryUpdaterButtonGroup {...defaultProps} />);

    const copyButton = screen.getByTitle('Copy Summary');
    await user.click(copyButton);

    expect(defaultProps.onCopy).toHaveBeenCalledTimes(1);
    expect(mockTrackButtonClick).toHaveBeenCalledWith(
      'copy_summary',
      'meeting_details'
    );
  });

  /**
   * Verifies that the Copy button is disabled when hasSummary is false.
   */
  it('should disable the Copy button when hasSummary is false', () => {
    render(<SummaryUpdaterButtonGroup {...defaultProps} hasSummary={false} />);

    const copyButton = screen.getByTitle('Copy Summary');
    expect(copyButton).toBeDisabled();
  });
});
