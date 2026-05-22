import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';

// ── Type helpers ──────────────────────────────────────────────────────

interface InvokeCall {
  command: string;
  args: Record<string, unknown>;
}

// ── Mocks ─────────────────────────────────────────────────────────────

// Mock Tauri invoke
const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock sonner toast
const mockToastError = vi.fn();
const mockToastInfo = vi.fn();
const mockToastSuccess = vi.fn();
vi.mock('sonner', () => ({
  toast: {
    error: (...args: unknown[]) => mockToastError(...args),
    info: (...args: unknown[]) => mockToastInfo(...args),
    success: (...args: unknown[]) => mockToastSuccess(...args),
  },
}));

// Mock Analytics
const mockTrackSummaryGenerationStarted = vi.fn().mockResolvedValue(undefined);
const mockTrackSummaryGenerationCompleted = vi.fn().mockResolvedValue(undefined);
const mockTrackCustomPromptUsed = vi.fn().mockResolvedValue(undefined);
const mockTrackButtonClick = vi.fn().mockResolvedValue(undefined);
vi.mock('@/lib/analytics', () => ({
  default: {
    trackSummaryGenerationStarted: (...args: unknown[]) => mockTrackSummaryGenerationStarted(...args),
    trackSummaryGenerationCompleted: (...args: unknown[]) => mockTrackSummaryGenerationCompleted(...args),
    trackCustomPromptUsed: (...args: unknown[]) => mockTrackCustomPromptUsed(...args),
    trackButtonClick: (...args: unknown[]) => mockTrackButtonClick(...args),
  },
}));

// Mock utils
vi.mock('@/lib/utils', () => ({
  isOllamaNotInstalledError: vi.fn().mockReturnValue(false),
  cn: (...args: unknown[]) => args.filter(Boolean).join(' '),
}));

// Mock builtin-ai
vi.mock('@/lib/builtin-ai', () => ({
  BuiltInModelInfo: {},
}));

// Mock SidebarProvider
const mockStartSummaryPolling = vi.fn();
const mockStopSummaryPolling = vi.fn();
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    startSummaryPolling: mockStartSummaryPolling,
    stopSummaryPolling: mockStopSummaryPolling,
  }),
}));

// ── Helpers ───────────────────────────────────────────────────────────

/**
 * Creates default props for the useSummaryGeneration hook.
 */
function createDefaultProps() {
  return {
    meeting: {
      id: 'meeting-123',
      title: 'Test Meeting',
      created_at: new Date().toISOString(),
    },
    transcripts: [],
    modelConfig: {
      provider: 'ollama' as const,
      model: 'llama3.2:latest',
      whisperModel: 'large-v3',
      apiKey: null,
      ollamaEndpoint: null,
    },
    isModelConfigLoading: false,
    selectedTemplate: 'standard_meeting',
    onMeetingUpdated: vi.fn().mockResolvedValue(undefined),
    updateMeetingTitle: vi.fn(),
    setAiSummary: vi.fn(),
    onOpenModelSettings: vi.fn(),
  };
}

/**
 * Sets up mockInvoke to handle api_process_transcript by returning
 * a valid process_id, and captures the startSummaryPolling callback.
 */
function setupProcessTranscriptMock() {
  mockInvoke.mockImplementation(async (command: string, args?: any) => {
    if (command === 'api_process_transcript') {
      return { process_id: 'proc-abc-123' };
    }
    if (command === 'api_get_meeting_transcripts') {
      return {
        transcripts: [
          {
            id: 't1',
            text: 'Hello from the database transcript',
            timestamp: '14:30:00',
            audio_start_time: 10,
          },
          {
            id: 't2',
            text: 'Second transcript segment',
            timestamp: '14:31:00',
            audio_start_time: 70,
          },
        ],
        total_count: 2,
        has_more: false,
      };
    }
    if (command === 'get_ollama_models') {
      return [{ name: 'llama3.2:latest' }];
    }
    if (command === 'api_cancel_summary') {
      return null;
    }
    return null;
  });
}

// ── Test Suite ────────────────────────────────────────────────────────

describe('useSummaryGeneration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupProcessTranscriptMock();
  });

  // ── Initial State ─────────────────────────────────────────────────

  /**
   * Verifies the hook returns the correct initial state with idle status,
   * no error, and all handler functions defined.
   */
  it('should return initial state with idle status and handler functions', () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    expect(result.current.summaryStatus).toBe('idle');
    expect(result.current.summaryError).toBeNull();
    expect(typeof result.current.handleGenerateSummary).toBe('function');
    expect(typeof result.current.handleRegenerateSummary).toBe('function');
    expect(typeof result.current.handleStopGeneration).toBe('function');
    expect(typeof result.current.getSummaryStatusMessage).toBe('function');
  });

  // ── getSummaryStatusMessage ────────────────────────────────────────

  /**
   * Verifies that getSummaryStatusMessage returns the correct message
   * for each summary status value.
   */
  it.each([
    ['processing', 'Processing transcript...'],
    ['summarizing', 'Generating summary...'],
    ['regenerating', 'Regenerating summary...'],
    ['completed', 'Summary completed'],
    ['error', 'Error generating summary'],
    ['idle', ''],
  ] as const)('should return correct status message for status: %s', (status, expectedMessage) => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    expect(result.current.getSummaryStatusMessage(status as any)).toBe(expectedMessage);
  });

  // ── handleRegenerateSummary: uses cached originalTranscript ────────

  /**
   * Verifies that handleRegenerateSummary uses the cached originalTranscript
   * when it is available (i.e., after a previous summary generation in the
   * same session). It should NOT fetch transcripts from the database.
   *
   * The hook stores originalTranscript in state during the first generation
   * (via processSummary with isRegeneration=false). After the state update
   * and re-render, handleRegenerateSummary should use the cached value.
   */
  it('should use cached originalTranscript when available for regeneration', async () => {
    const props = createDefaultProps();
    const { result, rerender } = renderHook(() => useSummaryGeneration(props));

    // Step 1: Generate summary first to cache the originalTranscript
    await act(async () => {
      await result.current.handleGenerateSummary('');
    });

    // Force a re-render so the hook picks up the updated originalTranscript state
    rerender();

    vi.clearAllMocks();
    setupProcessTranscriptMock();

    // Step 2: Call handleRegenerateSummary — should use cached transcript
    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    // Should NOT have fetched transcripts from DB again since originalTranscript is cached
    const fetchCallsAfterRegen = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_get_meeting_transcripts'
    ).length;
    expect(fetchCallsAfterRegen).toBe(0);

    // Should have called api_process_transcript (via processSummary)
    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(1);
  });

  // ── handleRegenerateSummary: fetches from DB when no cache ─────────

  /**
   * Verifies that handleRegenerateSummary fetches transcripts from the
   * database when originalTranscript is empty (e.g., after page reload
   * or navigation). This covers the fallback path.
   */
  it('should fetch transcripts from database when originalTranscript is empty', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    // Directly call handleRegenerateSummary without prior generation
    // so originalTranscript is empty
    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    // Should have fetched transcripts from the database
    const fetchCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_get_meeting_transcripts'
    );
    expect(fetchCalls.length).toBeGreaterThan(0);

    // Should have called api_process_transcript with the fetched transcript text
    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(1);

    // Verify the transcript text contains content from the DB transcripts
    const processArgs = processTranscriptCalls[0][1] as any;
    expect(processArgs.text).toContain('Hello from the database transcript');
    expect(processArgs.text).toContain('Second transcript segment');
  });

  // ── handleRegenerateSummary: error toast when no transcripts ───────

  /**
   * Verifies that handleRegenerateSummary shows an error toast when
   * no transcripts are available from the database and no cached
   * transcript exists.
   */
  it('should show error toast when no transcripts are available for regeneration', async () => {
    // Override mock to return empty transcripts
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'api_get_meeting_transcripts') {
        return {
          transcripts: [],
          total_count: 0,
          has_more: false,
        };
      }
      return null;
    });

    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    // Should show error toast
    expect(mockToastError).toHaveBeenCalledWith(
      'No transcripts available for regeneration'
    );

    // Should NOT have called api_process_transcript
    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(0);
  });

  // ── processSummary called with isRegeneration: true ────────────────

  /**
   * Verifies that when handleRegenerateSummary calls processSummary,
   * it passes isRegeneration: true, which results in the status being
   * set to 'regenerating' and the toast showing 'Regenerating' text.
   */
  it('should call processSummary with isRegeneration true during regeneration', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    // Call handleRegenerateSummary (will fetch from DB since no cached transcript)
    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    // Verify the toast.info was called with 'Regenerating' (not 'Generating')
    expect(mockToastInfo).toHaveBeenCalledWith(
      expect.stringContaining('Regenerating'),
      expect.objectContaining({
        description: expect.any(String),
      })
    );
  });

  // ── handleRegenerateSummary: formats timestamps correctly ──────────

  /**
   * Verifies that handleRegenerateSummary formats transcript timestamps
   * as recording-relative [MM:SS] when audio_start_time is available.
   */
  it('should format timestamps as [MM:SS] when audio_start_time is available', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(1);

    const processArgs = processTranscriptCalls[0][1] as any;
    // audio_start_time: 10 => [00:10]
    expect(processArgs.text).toContain('[00:10]');
    // audio_start_time: 70 => [01:10]
    expect(processArgs.text).toContain('[01:10]');
  });

  // ── handleRegenerateSummary: passes correct meeting ID ─────────────

  /**
   * Verifies that handleRegenerateSummary passes the correct meeting ID
   * to both the transcript fetch and the process transcript calls.
   */
  it('should pass correct meeting ID to api_process_transcript', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    // Verify meeting ID in api_get_meeting_transcripts call
    const fetchCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_get_meeting_transcripts'
    );
    expect(fetchCalls.length).toBeGreaterThan(0);
    expect(fetchCalls[0][1]).toEqual(expect.objectContaining({
      meetingId: 'meeting-123',
    }));

    // Verify meeting ID in api_process_transcript call
    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(1);
    expect(processTranscriptCalls[0][1]).toEqual(expect.objectContaining({
      meetingId: 'meeting-123',
    }));
  });

  // ── handleRegenerateSummary: starts polling after process ──────────

  /**
   * Verifies that handleRegenerateSummary starts summary polling after
   * successfully invoking api_process_transcript.
   */
  it('should start summary polling after successful process transcript call', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleRegenerateSummary();
    });

    expect(mockStartSummaryPolling).toHaveBeenCalledWith(
      'meeting-123',
      'proc-abc-123',
      expect.any(Function)
    );
  });

  // ── handleStopGeneration ──────────────────────────────────────────

  /**
   * Verifies that handleStopGeneration calls the backend cancel API,
   * stops polling, resets status to idle, and shows an info toast.
   */
  it('should cancel generation, stop polling, and reset status when stopped', async () => {
    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleStopGeneration();
    });

    // Should call backend cancel
    const cancelCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_cancel_summary'
    );
    expect(cancelCalls.length).toBe(1);
    expect(cancelCalls[0][1]).toEqual({ meetingId: 'meeting-123' });

    // Should stop polling
    expect(mockStopSummaryPolling).toHaveBeenCalledWith('meeting-123');

    // Status should be idle
    expect(result.current.summaryStatus).toBe('idle');
    expect(result.current.summaryError).toBeNull();

    // Should show info toast
    expect(mockToastInfo).toHaveBeenCalledWith(
      'Summary generation stopped',
      expect.objectContaining({
        description: expect.any(String),
      })
    );
  });

  // ── handleGenerateSummary: waits when model config loading ─────────

  /**
   * Verifies that handleGenerateSummary shows an info toast and returns
   * early when isModelConfigLoading is true.
   */
  it('should show info toast and return early when model config is loading', async () => {
    const props = createDefaultProps();
    props.isModelConfigLoading = true;
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleGenerateSummary('');
    });

    expect(mockToastInfo).toHaveBeenCalledWith(
      'Loading model configuration, please wait...'
    );

    // Should NOT have called api_process_transcript
    const processTranscriptCalls = mockInvoke.mock.calls.filter(
      (call: any[]) => call[0] === 'api_process_transcript'
    );
    expect(processTranscriptCalls.length).toBe(0);
  });

  // ── handleGenerateSummary: error when no transcripts ───────────────

  /**
   * Verifies that handleGenerateSummary shows an error toast when
   * no transcripts are available in the database.
   */
  it('should show error toast when no transcripts available for generation', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'api_get_meeting_transcripts') {
        return {
          transcripts: [],
          total_count: 0,
          has_more: false,
        };
      }
      return null;
    });

    const props = createDefaultProps();
    const { result } = renderHook(() => useSummaryGeneration(props));

    await act(async () => {
      await result.current.handleGenerateSummary('');
    });

    expect(mockToastError).toHaveBeenCalledWith(
      'No transcripts available for summary'
    );
  });
});
