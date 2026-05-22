import { useState, useCallback } from 'react';
import { Transcript, Summary, SummaryDataResponse } from '@/types';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { CurrentMeeting, useSidebar } from '@/components/Sidebar/SidebarProvider';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import Analytics from '@/lib/analytics';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';

// Import shared validation utility (includes children validation + recursive children check)
import { isValidBlockNoteArray, sanitizeBlockNoteArray } from '@/lib/blocknote-validation';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

/**
 * Sanitizes summary data before passing it to the UI.
 *
 * Uses the shared blocknote-validation utility which validates `children`
 * recursively – the missing children check was the root cause of the
 * "Invalid array passed to renderSpec" error.
 *
 * If summary_json is entirely invalid after sanitisation, it is stripped so
 * the data falls through to markdown or legacy format rendering.
 */
function sanitizeSummaryData(data: any): any {
  if (!data || typeof data !== 'object') return data;

  if (data.summary_json && Array.isArray(data.summary_json)) {
    // Use shared utility (validates inline content + children recursively)
    if (!isValidBlockNoteArray(data.summary_json)) {
      const sanitized = sanitizeBlockNoteArray(data.summary_json);
      if (sanitized.length === 0) {
        console.warn(
          '[useSummaryGeneration] sanitizeSummaryData: all blocks invalid after sanitisation, stripping summary_json',
        );
        const { summary_json, ...rest } = data;
        return rest;
      }
      console.warn(
        '[useSummaryGeneration] sanitizeSummaryData: repaired',
        data.summary_json.length - sanitized.length,
        'invalid block(s)',
      );
      return { ...data, summary_json: sanitized };
    }
  }

  return data;
}

interface UseSummaryGenerationProps {
  meeting: any;
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  isModelConfigLoading: boolean;
  selectedTemplate: string;
  onMeetingUpdated?: () => Promise<void>;
  updateMeetingTitle: (title: string) => void;
  setAiSummary: (summary: Summary | SummaryDataResponse | null) => void;
  onOpenModelSettings?: () => void;
}

export function useSummaryGeneration({
  meeting,
  transcripts,
  modelConfig,
  isModelConfigLoading,
  selectedTemplate,
  onMeetingUpdated,
  updateMeetingTitle,
  setAiSummary,
  onOpenModelSettings,
}: UseSummaryGenerationProps) {
  const [summaryStatus, setSummaryStatus] = useState<SummaryStatus>('idle');
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const [originalTranscript, setOriginalTranscript] = useState<string>('');

  const { startSummaryPolling, stopSummaryPolling } = useSidebar();

  // Helper to get status message
  const getSummaryStatusMessage = useCallback((status: SummaryStatus) => {
    switch (status) {
      case 'processing':
        return 'Processing transcript...';
      case 'summarizing':
        return 'Generating summary...';
      case 'regenerating':
        return 'Regenerating summary...';
      case 'completed':
        return 'Summary completed';
      case 'error':
        return 'Error generating summary';
      default:
        return '';
    }
  }, []);

  // Unified summary processing logic
  const processSummary = useCallback(async ({
    transcriptText,
    customPrompt = '',
    isRegeneration = false,
  }: {
    transcriptText: string;
    customPrompt?: string;
    isRegeneration?: boolean;
  }) => {
    setSummaryStatus(isRegeneration ? 'regenerating' : 'processing');
    setSummaryError(null);

    try {
      if (!transcriptText.trim()) {
        throw new Error('No transcript text available. Please add some text first.');
      }

      if (!isRegeneration) {
        setOriginalTranscript(transcriptText);
      }

      console.log('Processing transcript with template:', selectedTemplate);

      // Calculate time since recording
      const timeSinceRecording = (Date.now() - new Date(meeting.created_at).getTime()) / 60000; // minutes

      // Track summary generation started
      await Analytics.trackSummaryGenerationStarted(
        modelConfig.provider,
        modelConfig.model,
        transcriptText.length,
        timeSinceRecording
      );

      // Track custom prompt usage if present
      if (customPrompt.trim().length > 0) {
        await Analytics.trackCustomPromptUsed(customPrompt.trim().length);
      }

      // Show toast notification for generation start
      toast.info(`${isRegeneration ? 'Regenerating' : 'Generating'} summary...`, {
        description: `Using ${modelConfig.provider}/${modelConfig.model}`,
        duration: 3000,
      });

      // Process transcript and get process_id
      let result: any;
      try {
        result = await invokeTauri('api_process_transcript', {
          text: transcriptText,
          model: modelConfig.provider,
          modelName: modelConfig.model,
          meetingId: meeting.id,
          chunkSize: 40000,
          overlap: 1000,
          customPrompt: customPrompt,
          templateId: selectedTemplate,
        });
      } catch (invokeError) {
        console.error('Failed to invoke api_process_transcript:', invokeError);
        const errorMsg = invokeError instanceof Error ? invokeError.message : String(invokeError);
        throw new Error(`Failed to start summary generation: ${errorMsg}`);
      }

      if (!result || !result.process_id) {
        throw new Error('Invalid response from summary engine: missing process_id');
      }

      const process_id = result.process_id;
      console.log('Process ID:', process_id);

      // Start global polling via context
      startSummaryPolling(meeting.id, process_id, async (pollingResult) => {
        console.log('Summary status:', pollingResult);

        // Handle cancellation
        if (pollingResult.status === 'cancelled') {
          console.log('Summary generation was cancelled');

          // Reload summary from database (backend has already restored from backup)
          try {
            const existingSummary = await invokeTauri('api_get_summary', {
              meetingId: meeting.id
            }) as any;

            if (existingSummary?.data) {
              console.log('Restored previous summary after cancellation');
              const safeData = sanitizeSummaryData(existingSummary.data);
              if (safeData) {
                setAiSummary(safeData);
                setSummaryStatus('completed');
              } else {
                console.warn('Restored summary data is invalid, resetting to idle');
                setSummaryStatus('idle');
              }
            } else {
              setSummaryStatus('idle');
            }
          } catch (error) {
            console.error('Failed to reload summary after cancellation:', error);
            setSummaryStatus('idle');
          }

          setSummaryError(null);
          return;
        }

        // Handle errors
        if (pollingResult.status === 'error' || pollingResult.status === 'failed') {
          console.error('Backend returned error:', pollingResult.error);
          const errorMessage = pollingResult.error || `Summary ${isRegeneration ? 'regeneration' : 'generation'} failed`;

          // If this was a regeneration, try to restore previous summary from database
          if (isRegeneration) {
            try {
              const existingSummary = await invokeTauri('api_get_summary', {
                meetingId: meeting.id
              }) as any;

              if (existingSummary?.data) {
                console.log('Restored previous summary after regeneration failure');
                const safeData = sanitizeSummaryData(existingSummary.data);
                if (safeData) {
                  setAiSummary(safeData);
                  setSummaryStatus('completed');
                  setSummaryError(null);

                  // Show error toast with restoration message
                  toast.error(`Failed to regenerate summary`, {
                    description: `${errorMessage}. Your previous summary has been restored.`,
                  });

                  await Analytics.trackSummaryGenerationCompleted(
                    modelConfig.provider,
                    modelConfig.model,
                    false,
                    undefined,
                    errorMessage
                  );
                  return;
                }
              }
            } catch (error) {
              console.error('Failed to reload summary after error:', error);
            }
          }

          // Continue with normal error handling if not regeneration or reload failed
          setSummaryError(errorMessage);
          setSummaryStatus('error');

          // Check if this is a "model is required" error
          const isModelRequiredError = errorMessage.includes('model is required') ||
            errorMessage.includes('"model":"required"') ||
            errorMessage.toLowerCase().includes('model') && errorMessage.toLowerCase().includes('required');

          // Show error toast
          toast.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary`, {
            description: errorMessage.includes('Connection refused')
              ? 'Could not connect to LLM service. Please ensure Ollama or your configured LLM provider is running.'
              : errorMessage,
          });

          // Auto-open model settings modal if model is missing
          if (isModelRequiredError && onOpenModelSettings) {
            console.log('🔧 Model required error detected, opening model settings...');
            onOpenModelSettings();
          }

          await Analytics.trackSummaryGenerationCompleted(
            modelConfig.provider,
            modelConfig.model,
            false,
            undefined,
            errorMessage
          );
          return;
        }

        // Handle successful completion
        if (pollingResult.status === 'completed' && pollingResult.data) {
          console.log('Summary generation completed, data keys:', Object.keys(pollingResult.data));

          // Update meeting title if available
          const meetingName = pollingResult.data.MeetingName || pollingResult.meetingName;
          if (meetingName) {
            updateMeetingTitle(meetingName);
          }

          // Check if backend returned markdown format (new flow)
          // The Rust backend stores: { "markdown": "..." }
          if (pollingResult.data.markdown && typeof pollingResult.data.markdown === 'string' && pollingResult.data.markdown.trim().length > 0) {
            console.log('Received markdown format from backend, length:', pollingResult.data.markdown.length);
            // Pass the entire data object (which has { markdown: "..." }) as SummaryDataResponse
            // BlockNoteSummaryView.detectSummaryFormat will pick up the 'markdown' format
            const summaryDataResponse = { markdown: pollingResult.data.markdown };
            setAiSummary(summaryDataResponse as any);
            setSummaryStatus('completed');

            // Show success toast
            toast.success('Summary generated successfully!', {
              description: 'Your meeting summary is ready',
              duration: 4000,
            });

            if (meetingName && onMeetingUpdated) {
              await onMeetingUpdated();
            }

            await Analytics.trackSummaryGenerationCompleted(
              modelConfig.provider,
              modelConfig.model,
              true
            );
            return;
          }

          // Check if it has summary_json (BlockNote format from a previous save)
          if (pollingResult.data.summary_json && Array.isArray(pollingResult.data.summary_json) && pollingResult.data.summary_json.length > 0) {
            // ALWAYS sanitize before passing to UI, even if validation passes.
            // This ensures any edge cases that might slip through are caught.
            const sanitizedBlocks = sanitizeBlockNoteArray(pollingResult.data.summary_json);
            
            if (sanitizedBlocks.length > 0) {
              console.log('Summary data sanitized, blocks:', sanitizedBlocks.length);
              setAiSummary({
                ...pollingResult.data,
                summary_json: sanitizedBlocks
              } as any);
              setSummaryStatus('completed');

              toast.success('Summary generated successfully!', {
                description: 'Your meeting summary is ready',
                duration: 4000,
              });

              if (meetingName && onMeetingUpdated) {
                await onMeetingUpdated();
              }

              await Analytics.trackSummaryGenerationCompleted(
                modelConfig.provider,
                modelConfig.model,
                true
              );
              return;
            } else {
              // Blocks failed validation – sanitise using shared utility
              const sanitizedData = sanitizeSummaryData(pollingResult.data);
              if (sanitizedData && sanitizedData.summary_json && isValidBlockNoteArray(sanitizedData.summary_json)) {
                console.log('[useSummaryGeneration] Summary data sanitized successfully, blocks:', sanitizedData.summary_json.length);
                setAiSummary(sanitizedData as any);
                setSummaryStatus('completed');

                toast.success('Summary generated successfully!', {
                  description: 'Your meeting summary is ready',
                  duration: 4000,
                });

                if (meetingName && onMeetingUpdated) {
                  await onMeetingUpdated();
                }

                await Analytics.trackSummaryGenerationCompleted(
                  modelConfig.provider,
                  modelConfig.model,
                  true
                );
                return;
              }
              // If sanitisation removed all blocks, fall through to legacy/markdown handling
              console.warn('[useSummaryGeneration] summary_json invalid and could not be sanitized, falling through to legacy format');
            }
          }

          // Legacy format handling
          const { MeetingName, _section_order, markdown: _md, summary_json: _sj, ...restData } = pollingResult.data;

          // Check if there are any valid legacy sections
          const legacySections = Object.entries(restData).filter(([, section]) => {
            return section && typeof section === 'object' && 'title' in (section as any) && 'blocks' in (section as any);
          });

          if (legacySections.length === 0) {
            console.error('Summary completed but no valid sections found in data');
            setSummaryError('Summary generation completed but returned empty content.');
            setSummaryStatus('error');

            await Analytics.trackSummaryGenerationCompleted(
              modelConfig.provider,
              modelConfig.model,
              false,
              undefined,
              'Empty summary generated'
            );
            return;
          }

          // Format legacy summary data
          const formattedSummary: Summary = {};
          const sectionKeys = _section_order || Object.keys(restData);

          for (const key of sectionKeys) {
            try {
              const section = restData[key];
              if (section && typeof section === 'object' && 'title' in section && 'blocks' in section) {
                const typedSection = section as { title?: string; blocks?: any[] };

                if (Array.isArray(typedSection.blocks)) {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: typedSection.blocks.map((block: any) => ({
                      ...block,
                      color: block?.color || 'default',
                      content: typeof block?.content === 'string' ? block.content.trim() : ''
                    }))
                  };
                } else {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: []
                  };
                }
              }
            } catch (error) {
              console.warn(`Error processing section ${key}:`, error);
            }
          }

          if (Object.keys(formattedSummary).length === 0) {
            console.error('Summary completed but all sections were empty after formatting');
            setSummaryError('Summary generation completed but returned empty content.');
            setSummaryStatus('error');
            return;
          }

          setAiSummary(formattedSummary);
          setSummaryStatus('completed');

          // Show success toast
          toast.success('Summary generated successfully!', {
            description: 'Your meeting summary is ready',
            duration: 4000,
          });

          await Analytics.trackSummaryGenerationCompleted(
            modelConfig.provider,
            modelConfig.model,
            true
          );

          if (meetingName && onMeetingUpdated) {
            await onMeetingUpdated();
          }
        }
      });
    } catch (error) {
      console.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary:`, error);
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      setSummaryError(errorMessage);
      setSummaryStatus('error');
      // Note: We don't clear the summary here because the backend has already restored from backup

      toast.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary`, {
        description: errorMessage,
      });

      await Analytics.trackSummaryGenerationCompleted(
        modelConfig.provider,
        modelConfig.model,
        false,
        undefined,
        errorMessage
      );
    }
  }, [
    meeting.id,
    meeting.created_at,
    modelConfig,
    selectedTemplate,
    startSummaryPolling,
    setAiSummary,
    updateMeetingTitle,
    onMeetingUpdated,
  ]);

  // Helper function to fetch ALL transcripts for summary generation
  const fetchAllTranscripts = useCallback(async (meetingId: string): Promise<Transcript[]> => {
    try {
      console.log('📊 Fetching all transcripts for meeting:', meetingId);

      // First, get total count by fetching first page
      const firstPage = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: 1,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      const totalCount = firstPage.total_count;
      console.log(`📊 Total transcripts in database: ${totalCount}`);

      if (totalCount === 0) {
        return [];
      }

      // Fetch all transcripts in one call
      const allData = await invokeTauri('api_get_meeting_transcripts', {
        meetingId,
        limit: totalCount,
        offset: 0,
      }) as { transcripts: Transcript[]; total_count: number; has_more: boolean };

      console.log(`✅ Fetched ${allData.transcripts.length} transcripts from database`);
      return allData.transcripts;
    } catch (error) {
      console.error('❌ Error fetching all transcripts:', error);
      toast.error('Failed to fetch transcripts for summary generation');
      return [];
    }
  }, []);

  // Public API: Generate summary from transcripts
  const handleGenerateSummary = useCallback(async (customPrompt: string = '') => {
    // Check if model config is still loading
    if (isModelConfigLoading) {
      console.log('⏳ Model configuration is still loading, please wait...');
      toast.info('Loading model configuration, please wait...');
      return;
    }

    // CHANGE: Fetch ALL transcripts from database, not from pagination state
    console.log('📊 Fetching all transcripts for summary generation...');
    const allTranscripts = await fetchAllTranscripts(meeting.id);

    if (!allTranscripts.length) {
      const error_msg = 'No transcripts available for summary';
      console.log(error_msg);
      toast.error(error_msg);
      return;
    }

    console.log(`✅ Proceeding with ${allTranscripts.length} transcripts`);

    console.log('🚀 Starting summary generation with config:', {
      provider: modelConfig.provider,
      model: modelConfig.model,
      template: selectedTemplate
    });

    // Check if Ollama provider has models available
    if (modelConfig.provider === 'ollama') {
      try {
        const endpoint = modelConfig.ollamaEndpoint || null;
        const models = await invokeTauri('get_ollama_models', { endpoint }) as any[];

        if (!models || models.length === 0) {
          toast.error(
            'No Ollama models found. Please download gemma3:1b from Model Settings.',
            { duration: 5000 }
          );
          return;
        }
      } catch (error) {
        console.error('Error checking Ollama models:', error);
        const errorMessage = error instanceof Error ? error.message : String(error);

        if (isOllamaNotInstalledError(errorMessage)) {
          // Ollama is not installed - show specific message with download link
          toast.error(
            'Ollama is not installed',
            {
              description: 'Please download and install Ollama to use local models.',
              duration: 7000,
              action: {
                label: 'Download',
                onClick: () => invokeTauri('open_external_url', { url: 'https://ollama.com/download' })
              }
            }
          );
        } else {
          // Other error - generic message
          toast.error(
            'Failed to check Ollama models. Please ensure Ollama is running and download a model from Settings.',
            { duration: 5000 }
          );
        }
        return;
      }
    }

    // Check if built-in AI provider has models available
    if (modelConfig.provider === 'builtin-ai') {
      try {
        const selectedModel = modelConfig.model;

        if (!selectedModel) {
          toast.error('No built-in AI model selected', {
            description: 'Please select a model in settings',
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return;
        }

        // Check model readiness with filesystem refresh
        const isReady = await invokeTauri<boolean>('builtin_ai_is_model_ready', {
          modelName: selectedModel,
          refresh: true,
        });

        if (!isReady) {
          // Get detailed model status
          const modelInfo = await invokeTauri<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
            modelName: selectedModel,
          });

          if (modelInfo) {
            const status = modelInfo.status;

            if (status.type === 'downloading') {
              toast.info('Model download in progress', {
                description: `${selectedModel} is downloading (${status.progress}%). Please wait until download completes.`,
                duration: 5000,
              });
              return;
            }

            if (status.type === 'not_downloaded') {
              toast.error('Built-in AI model not downloaded', {
                description: `${selectedModel} needs to be downloaded. Please download it in model settings.`,
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return;
            }

            if (status.type === 'corrupted' || status.type === 'error') {
              const errorDesc = status.type === 'error'
                ? status.Error || 'The model file has an error'
                : 'The model file is corrupted';
              toast.error('Built-in AI model not available', {
                description: `${errorDesc}. Please check model settings.`,
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return;
            }
          }

          // Fallback if we couldn't get model info
          toast.error('Built-in AI model not ready', {
            description: 'Please ensure the model is downloaded in settings',
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return;
        }

        // Model is ready, continue to backend call
      } catch (error) {
        console.error('Error validating built-in AI model:', error);
        toast.error('Failed to validate built-in AI model', {
          description: error instanceof Error ? error.message : String(error),
          duration: 5000,
        });
        return;
      }
    }

    // Format timestamps as recording-relative [MM:SS] instead of wall-clock time
    const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
      if (seconds === undefined) {
        // For old transcripts without audio_start_time, use wall-clock time
        return fallbackTimestamp;
      }
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    const fullTranscript = allTranscripts
      .map(t => `${formatTime(t.audio_start_time, t.timestamp)} ${t.text}`)
      .join('\n');

    await processSummary({ transcriptText: fullTranscript, customPrompt });
  }, [meeting.id, fetchAllTranscripts, processSummary, modelConfig, isModelConfigLoading, selectedTemplate]);

  // Public API: Regenerate summary by re-fetching transcripts from database
  const handleRegenerateSummary = useCallback(async () => {
    // If we have the original transcript from the current session, use it
    if (originalTranscript.trim()) {
      await processSummary({
        transcriptText: originalTranscript,
        isRegeneration: true,
      });
      return;
    }

    // Otherwise, fetch all transcripts from the database (handles page-reload / navigation case)
    console.log('📊 No cached transcript available for regeneration, fetching from database...');
    const allTranscripts = await fetchAllTranscripts(meeting.id);

    if (!allTranscripts.length) {
      const errorMsg = 'No transcripts available for regeneration';
      console.error(errorMsg);
      toast.error(errorMsg);
      return;
    }

    // Format timestamps consistently with handleGenerateSummary
    const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
      if (seconds === undefined) {
        return fallbackTimestamp;
      }
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    const fullTranscript = allTranscripts
      .map(t => `${formatTime(t.audio_start_time, t.timestamp)} ${t.text}`)
      .join('\n');

    console.log(`✅ Fetched ${allTranscripts.length} transcripts for regeneration`);

    await processSummary({
      transcriptText: fullTranscript,
      isRegeneration: true,
    });
  }, [originalTranscript, processSummary, fetchAllTranscripts, meeting.id]);

  // Public API: Stop ongoing summary generation
  const handleStopGeneration = useCallback(async () => {
    console.log('Stopping summary generation for meeting:', meeting.id);

    try {
      // Call backend to cancel the summary generation
      await invokeTauri('api_cancel_summary', {
        meetingId: meeting.id
      });
      console.log('✓ Backend cancellation request sent for meeting:', meeting.id);
    } catch (error) {
      console.error('Failed to cancel summary generation:', error);
      // Continue with frontend cleanup even if backend call fails
    }

    // Stop polling
    stopSummaryPolling(meeting.id);

    // Reset status to idle
    setSummaryStatus('idle');
    setSummaryError(null);

    // Show toast notification
    toast.info('Summary generation stopped', {
      description: 'You can generate a new summary anytime',
      duration: 3000,
    });
  }, [meeting.id, stopSummaryPolling]);

  return {
    summaryStatus,
    summaryError,
    handleGenerateSummary,
    handleRegenerateSummary,
    handleStopGeneration,
    getSummaryStatusMessage,
  };
}
