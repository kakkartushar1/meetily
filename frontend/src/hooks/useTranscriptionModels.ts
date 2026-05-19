import { useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { type CustomModelCatalogEntry } from '../constants/modelCatalog';

export interface RawModelInfo {
  name: string;
  size_mb: number;
  status: 'Available' | 'Missing' | { Downloading: { progress: number } } | { Error: string };
}

export interface NemoRawModelInfo {
  model_id: string;
  filename: string;
  size_mb: number;
  label: string;
  description: string;
  status: 'Available' | 'Missing' | { Downloading: { progress: number } } | { Error: string };
}

export interface ModelOption {
  provider: 'whisper' | 'parakeet';
  name: string;
  displayName: string;
  size_mb: number;
  runtime?: 'localWhisper' | 'parakeet' | 'nemo';
  isCustom?: boolean;
}

interface TranscriptModelConfig {
  provider?: string;
  model?: string;
}

/**
 * Custom hook for fetching and managing transcription models (Whisper and Parakeet).
 *
 * This hook centralizes the model fetching logic that was previously duplicated
 * in ImportAudioDialog and RetranscribeDialog components.
 *
 * @param transcriptModelConfig - User's saved model configuration from context
 * @returns Object containing available models, selected model key, loading state, and fetch function
 */
export function useTranscriptionModels(transcriptModelConfig: TranscriptModelConfig | undefined) {
  const [availableModels, setAvailableModels] = useState<ModelOption[]>([]);
  const [selectedModelKey, setSelectedModelKey] = useState<string>('');
  const [loadingModels, setLoadingModels] = useState(false);
  // Track whether the user has manually changed the model selection
  const userSelectedRef = useRef(false);

  // Wrap setSelectedModelKey to track user-initiated changes
  const setSelectedModelKeyWithTracking = useCallback((key: string) => {
    userSelectedRef.current = true;
    setSelectedModelKey(key);
  }, []);

  const fetchModels = useCallback(async () => {
    setLoadingModels(true);
    const allModels: ModelOption[] = [];

    // Fetch Whisper models
    try {
      const whisperModels = await invoke<RawModelInfo[]>('whisper_get_available_models');
      const availableWhisper = whisperModels
        .filter((m) => m.status === 'Available')
        .map((m) => ({
          provider: 'whisper' as const,
          name: m.name,
          displayName: `🏠 Whisper: ${m.name}`,
          size_mb: m.size_mb,
        }));
      allModels.push(...availableWhisper);
    } catch (err) {
      console.error('Failed to fetch Whisper models:', err);
    }

    // Fetch Parakeet ONNX models
    try {
      const parakeetModels = await invoke<RawModelInfo[]>('parakeet_get_available_models');
      const availableParakeet = parakeetModels
        .filter((m) => m.status === 'Available')
        .map((m) => ({
          provider: 'parakeet' as const,
          name: m.name,
          displayName: `⚡ Parakeet: ${m.name}`,
          size_mb: m.size_mb,
          runtime: 'parakeet' as const,
        }));
      allModels.push(...availableParakeet);
    } catch (err) {
      console.error('Failed to fetch Parakeet models:', err);
    }

    // Fetch NeMo models
    try {
      const nemoModels = await invoke<NemoRawModelInfo[]>('nemo_get_available_models');
      const availableNemo = nemoModels
        .filter((m) => m.status === 'Available')
        .map((m) => ({
          provider: 'parakeet' as const, // Same provider in DB
          name: m.model_id,
          displayName: `🎯 NeMo: ${m.label}`,
          size_mb: m.size_mb,
          runtime: 'nemo' as const,
        }));
      allModels.push(...availableNemo);
    } catch (err) {
      // NeMo engine may not be initialized yet - that's OK
      console.debug('NeMo models not available (expected if not initialized):', err);
    }

    // Fetch custom HuggingFace models
    try {
      const customModels = await invoke<CustomModelCatalogEntry[]>('parakeet_get_custom_models');
      const readyCustom = customModels
        .filter((m) => m.status === 'ready')
        .map((m) => ({
          provider: 'parakeet' as const,
          name: m.modelId,
          displayName: `🤗 Custom: ${m.label}`,
          size_mb: m.sizeMb,
          runtime: 'parakeet' as const,
          isCustom: true,
        }));
      allModels.push(...readyCustom);
    } catch (err) {
      console.debug('Custom models not available:', err);
    }

    setAvailableModels(allModels);

    // Set default model based on user's saved configuration
    const configuredProvider = transcriptModelConfig?.provider || '';
    const configuredModel = transcriptModelConfig?.model || '';

    // Try to match the configured model
    // Note: 'localWhisper' in config maps to 'whisper' provider in model list
    const configuredMatch = allModels.find(
      (m) =>
        (configuredProvider === 'localWhisper' && m.provider === 'whisper' && m.name === configuredModel) ||
        (configuredProvider === 'parakeet' && m.provider === 'parakeet' && m.name === configuredModel)
    );

    // Only set default model if user hasn't manually selected one
    if (!userSelectedRef.current) {
      if (configuredMatch) {
        // Use the configured model if available
        setSelectedModelKey(`${configuredMatch.provider}:${configuredMatch.name}`);
      } else if (allModels.length > 0) {
        // Fall back to first available model
        setSelectedModelKey(`${allModels[0].provider}:${allModels[0].name}`);
      }
    }

    setLoadingModels(false);
  }, [transcriptModelConfig]);

  // Reset user selection tracking (call when dialog opens fresh)
  const resetSelection = useCallback(() => {
    userSelectedRef.current = false;
  }, []);

  return {
    availableModels,
    selectedModelKey,
    setSelectedModelKey: setSelectedModelKeyWithTracking,
    loadingModels,
    fetchModels,
    resetSelection,
  };
}
