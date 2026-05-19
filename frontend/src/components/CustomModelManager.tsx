/**
 * Custom Model Manager Component
 *
 * Allows users to add custom HuggingFace STT models by repo ID or local path.
 * Supports model inspection, format detection, download with progress, and removal.
 *
 * Integrates with the Tauri backend via invoke commands:
 * - parakeet_inspect_huggingface_model
 * - parakeet_add_custom_model
 * - parakeet_add_local_model
 * - parakeet_remove_custom_model
 * - parakeet_get_custom_models
 * - parakeet_load_custom_model
 */

import React, { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { motion, AnimatePresence } from 'framer-motion';
import { toast } from 'sonner';
import {
  type CustomModelCatalogEntry,
  type ModelInspectionResult,
  type ModelFormat,
  getFormatLabel,
  getFormatIcon,
  isCustomModel,
} from '../constants/modelCatalog';

// ============================================================================
// TYPES
// ============================================================================

interface CustomModelManagerProps {
  /** Currently selected model ID */
  selectedModel: string;
  /** Callback when a custom model is selected */
  onModelSelect: (modelId: string) => void;
  /** Additional CSS classes */
  className?: string;
}

interface InspectionState {
  loading: boolean;
  result: ModelInspectionResult | null;
  error: string | null;
}

// ============================================================================
// COMPONENT
// ============================================================================

export function CustomModelManager({
  selectedModel,
  onModelSelect,
  className = '',
}: CustomModelManagerProps) {
  // State
  const [customModels, setCustomModels] = useState<CustomModelCatalogEntry[]>([]);
  const [showAddForm, setShowAddForm] = useState(false);
  const [inputMode, setInputMode] = useState<'huggingface' | 'local'>('huggingface');
  const [repoIdInput, setRepoIdInput] = useState('');
  const [localPathInput, setLocalPathInput] = useState('');
  const [labelInput, setLabelInput] = useState('');
  const [inspection, setInspection] = useState<InspectionState>({
    loading: false,
    result: null,
    error: null,
  });
  const [addingModel, setAddingModel] = useState(false);
  const [loadingModels, setLoadingModels] = useState(true);

  // Refs
  const inspectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // ── Load custom models on mount ──────────────────────────────────────
  const fetchCustomModels = useCallback(async () => {
    try {
      setLoadingModels(true);
      const models = await invoke<CustomModelCatalogEntry[]>('parakeet_get_custom_models');
      setCustomModels(models);
    } catch (err) {
      console.error('Failed to fetch custom models:', err);
    } finally {
      setLoadingModels(false);
    }
  }, []);

  useEffect(() => {
    fetchCustomModels();
  }, [fetchCustomModels]);

  // ── Event listeners for download progress ────────────────────────────
  useEffect(() => {
    let unlistenProgress: (() => void) | null = null;
    let unlistenComplete: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;

    const setupListeners = async () => {
      unlistenProgress = await listen<{
        modelId: string;
        overallPercent: number;
        currentFile: string;
        speedMbps: number;
      }>('custom-model-download-progress', (event) => {
        const { modelId, overallPercent } = event.payload;
        setCustomModels((prev) =>
          prev.map((m) =>
            m.modelId === modelId
              ? { ...m, status: { downloading: { progress: overallPercent } } }
              : m
          )
        );
      });

      unlistenComplete = await listen<{ modelId: string }>(
        'custom-model-download-complete',
        (event) => {
          const { modelId } = event.payload;
          setCustomModels((prev) =>
            prev.map((m) =>
              m.modelId === modelId ? { ...m, status: 'ready' } : m
            )
          );
          toast.success('Custom model ready!', {
            description: `Model ${modelId} downloaded successfully`,
            duration: 4000,
          });
        }
      );

      unlistenError = await listen<{ modelId: string; error: string }>(
        'custom-model-download-error',
        (event) => {
          const { modelId, error } = event.payload;
          setCustomModels((prev) =>
            prev.map((m) =>
              m.modelId === modelId ? { ...m, status: { error } } : m
            )
          );
          toast.error('Download failed', {
            description: error,
            duration: 6000,
          });
        }
      );
    };

    setupListeners();

    return () => {
      if (unlistenProgress) unlistenProgress();
      if (unlistenComplete) unlistenComplete();
      if (unlistenError) unlistenError();
    };
  }, []);

  // ── Inspect HuggingFace model (debounced) ────────────────────────────
  const inspectModel = useCallback(async (repoId: string) => {
    if (!repoId.trim() || !repoId.includes('/')) {
      setInspection({ loading: false, result: null, error: null });
      return;
    }

    setInspection({ loading: true, result: null, error: null });

    try {
      const result = await invoke<ModelInspectionResult>(
        'parakeet_inspect_huggingface_model',
        { repoId: repoId.trim() }
      );
      setInspection({ loading: false, result, error: null });
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      setInspection({ loading: false, result: null, error: errorMsg });
    }
  }, []);

  // Debounce inspection when repo ID changes
  useEffect(() => {
    if (inspectTimeoutRef.current) {
      clearTimeout(inspectTimeoutRef.current);
    }

    if (inputMode === 'huggingface' && repoIdInput.includes('/')) {
      inspectTimeoutRef.current = setTimeout(() => {
        inspectModel(repoIdInput);
      }, 800);
    }

    return () => {
      if (inspectTimeoutRef.current) {
        clearTimeout(inspectTimeoutRef.current);
      }
    };
  }, [repoIdInput, inputMode, inspectModel]);

  // ── Add custom model ─────────────────────────────────────────────────
  const handleAddModel = useCallback(async () => {
    setAddingModel(true);

    try {
      if (inputMode === 'huggingface') {
        if (!repoIdInput.trim() || !repoIdInput.includes('/')) {
          toast.error('Invalid repo ID', {
            description: 'Please enter a valid HuggingFace repo ID (e.g., "nvidia/parakeet-tdt-0.6b")',
          });
          return;
        }

        await invoke('parakeet_add_custom_model', {
          repoId: repoIdInput.trim(),
          label: labelInput.trim() || null,
        });

        toast.success('Model added!', {
          description: `Downloading ${repoIdInput}...`,
          duration: 5000,
        });
      } else {
        if (!localPathInput.trim()) {
          toast.error('Invalid path', {
            description: 'Please enter a valid local directory path',
          });
          return;
        }

        await invoke('parakeet_add_local_model', {
          path: localPathInput.trim(),
          label: labelInput.trim() || null,
        });

        toast.success('Local model added!', {
          description: 'Model is ready to use',
          duration: 4000,
        });
      }

      // Reset form and refresh
      setRepoIdInput('');
      setLocalPathInput('');
      setLabelInput('');
      setInspection({ loading: false, result: null, error: null });
      setShowAddForm(false);
      await fetchCustomModels();
    } catch (err) {
      const errorMsg = err instanceof Error ? err.message : String(err);
      toast.error('Failed to add model', {
        description: errorMsg,
        duration: 6000,
      });
    } finally {
      setAddingModel(false);
    }
  }, [inputMode, repoIdInput, localPathInput, labelInput, fetchCustomModels]);

  // ── Remove custom model ──────────────────────────────────────────────
  const handleRemoveModel = useCallback(
    async (modelId: string) => {
      try {
        await invoke('parakeet_remove_custom_model', { modelId });
        toast.success('Model removed', { duration: 3000 });

        if (selectedModel === modelId) {
          onModelSelect('');
        }

        await fetchCustomModels();
      } catch (err) {
        const errorMsg = err instanceof Error ? err.message : String(err);
        toast.error('Failed to remove model', {
          description: errorMsg,
          duration: 4000,
        });
      }
    },
    [selectedModel, onModelSelect, fetchCustomModels]
  );

  // ── Select custom model ──────────────────────────────────────────────
  const handleSelectModel = useCallback(
    async (modelId: string) => {
      try {
        await invoke('parakeet_load_custom_model', { modelId });
        onModelSelect(modelId);
        toast.success('Custom model loaded', { duration: 3000 });
      } catch (err) {
        const errorMsg = err instanceof Error ? err.message : String(err);
        toast.error('Failed to load model', {
          description: errorMsg,
          duration: 4000,
        });
      }
    },
    [onModelSelect]
  );

  // ── Helper: get status display ───────────────────────────────────────
  const getStatusDisplay = (status: CustomModelCatalogEntry['status']) => {
    if (status === 'pending') return { text: 'Pending', color: 'text-yellow-600', icon: '⏳' };
    if (status === 'converting') return { text: 'Converting...', color: 'text-blue-600', icon: '🔄' };
    if (status === 'ready') return { text: 'Ready', color: 'text-green-600', icon: '✅' };
    if (typeof status === 'object' && 'downloading' in status) {
      return {
        text: `Downloading ${status.downloading.progress}%`,
        color: 'text-blue-600',
        icon: '⬇️',
      };
    }
    if (typeof status === 'object' && 'error' in status) {
      return { text: 'Error', color: 'text-red-600', icon: '❌' };
    }
    return { text: 'Unknown', color: 'text-gray-500', icon: '❓' };
  };

  // ── Render ────────────────────────────────────────────────────────────
  return (
    <div className={`space-y-4 ${className}`}>
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-gray-700">
          🤗 Custom HuggingFace Models
        </h3>
        <button
          type="button"
          onClick={() => setShowAddForm(!showAddForm)}
          className="text-xs px-3 py-1.5 rounded-md bg-blue-50 text-blue-700 hover:bg-blue-100 transition-colors font-medium"
          aria-label={showAddForm ? 'Cancel adding model' : 'Add custom model'}
        >
          {showAddForm ? 'Cancel' : '+ Add Model'}
        </button>
      </div>

      {/* Add Model Form */}
      <AnimatePresence>
        {showAddForm && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            <div className="bg-gray-50 rounded-lg p-4 space-y-3 border border-gray-200">
              {/* Input Mode Tabs */}
              <div className="flex gap-2" role="tablist" aria-label="Model source">
                <button
                  type="button"
                  role="tab"
                  aria-selected={inputMode === 'huggingface'}
                  onClick={() => setInputMode('huggingface')}
                  className={`text-xs px-3 py-1.5 rounded-md transition-colors ${
                    inputMode === 'huggingface'
                      ? 'bg-blue-600 text-white'
                      : 'bg-white text-gray-600 hover:bg-gray-100'
                  }`}
                >
                  🤗 HuggingFace
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={inputMode === 'local'}
                  onClick={() => setInputMode('local')}
                  className={`text-xs px-3 py-1.5 rounded-md transition-colors ${
                    inputMode === 'local'
                      ? 'bg-blue-600 text-white'
                      : 'bg-white text-gray-600 hover:bg-gray-100'
                  }`}
                >
                  📁 Local Path
                </button>
              </div>

              {/* HuggingFace Repo ID Input */}
              {inputMode === 'huggingface' && (
                <div className="space-y-2">
                  <label htmlFor="hf-repo-id" className="text-xs font-medium text-gray-600">
                    HuggingFace Repo ID
                  </label>
                  <input
                    id="hf-repo-id"
                    type="text"
                    value={repoIdInput}
                    onChange={(e) => setRepoIdInput(e.target.value)}
                    placeholder="e.g., nvidia/parakeet-tdt-0.6b-v3"
                    className="w-full text-sm px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    aria-describedby="hf-repo-help"
                  />
                  <p id="hf-repo-help" className="text-xs text-gray-500">
                    Enter the HuggingFace model repository ID (organization/model-name)
                  </p>
                </div>
              )}

              {/* Local Path Input */}
              {inputMode === 'local' && (
                <div className="space-y-2">
                  <label htmlFor="local-path" className="text-xs font-medium text-gray-600">
                    Model Directory Path
                  </label>
                  <input
                    id="local-path"
                    type="text"
                    value={localPathInput}
                    onChange={(e) => setLocalPathInput(e.target.value)}
                    placeholder="e.g., C:\\Models\\my-custom-model"
                    className="w-full text-sm px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    aria-describedby="local-path-help"
                  />
                  <p id="local-path-help" className="text-xs text-gray-500">
                    Enter the full path to a directory containing model files (ONNX, Safetensors, etc.)
                  </p>
                </div>
              )}

              {/* Optional Label */}
              <div className="space-y-2">
                <label htmlFor="model-label" className="text-xs font-medium text-gray-600">
                  Custom Label (optional)
                </label>
                <input
                  id="model-label"
                  type="text"
                  value={labelInput}
                  onChange={(e) => setLabelInput(e.target.value)}
                  placeholder="e.g., My Fast Parakeet"
                  className="w-full text-sm px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                />
              </div>

              {/* Inspection Results */}
              {inputMode === 'huggingface' && (
                <InspectionPanel inspection={inspection} />
              )}

              {/* Add Button */}
              <button
                type="button"
                onClick={handleAddModel}
                disabled={
                  addingModel ||
                  (inputMode === 'huggingface' && (!repoIdInput.includes('/') || inspection.loading)) ||
                  (inputMode === 'local' && !localPathInput.trim())
                }
                className="w-full text-sm px-4 py-2 rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:bg-gray-300 disabled:cursor-not-allowed transition-colors font-medium"
                aria-busy={addingModel}
              >
                {addingModel ? 'Adding...' : inputMode === 'huggingface' ? 'Download & Add Model' : 'Add Local Model'}
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Custom Models List */}
      {loadingModels ? (
        <div className="animate-pulse space-y-2">
          <div className="h-16 bg-gray-100 rounded-lg" />
        </div>
      ) : customModels.length === 0 ? (
        <p className="text-xs text-gray-500 text-center py-4">
          No custom models added yet. Click "+ Add Model" to get started.
        </p>
      ) : (
        <div className="space-y-2">
          {customModels.map((model) => {
            const statusInfo = getStatusDisplay(model.status);
            const isSelected = selectedModel === model.modelId;
            const isReady = model.status === 'ready';
            const isDownloading =
              typeof model.status === 'object' && 'downloading' in model.status;

            return (
              <motion.div
                key={model.modelId}
                layout
                className={`relative rounded-lg border p-3 transition-all ${
                  isSelected
                    ? 'border-blue-500 bg-blue-50 ring-1 ring-blue-500'
                    : 'border-gray-200 bg-white hover:border-gray-300'
                }`}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm" aria-hidden="true">
                        {getFormatIcon(model.format)}
                      </span>
                      <h4 className="text-sm font-medium text-gray-900 truncate">
                        {model.label}
                      </h4>
                      <span
                        className={`text-xs px-1.5 py-0.5 rounded-full bg-gray-100 ${statusInfo.color}`}
                      >
                        {statusInfo.icon} {statusInfo.text}
                      </span>
                    </div>
                    <p className="text-xs text-gray-500 mt-1 truncate">
                      {model.repo || model.localPath || model.modelId}
                    </p>
                    <div className="flex items-center gap-3 mt-1">
                      <span className="text-xs text-gray-400">
                        {getFormatLabel(model.format)} • {model.sizeMb} MB
                      </span>
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-1 shrink-0">
                    {isReady && (
                      <button
                        type="button"
                        onClick={() => handleSelectModel(model.modelId)}
                        disabled={isSelected}
                        className={`text-xs px-2 py-1 rounded transition-colors ${
                          isSelected
                            ? 'bg-blue-600 text-white cursor-default'
                            : 'bg-green-50 text-green-700 hover:bg-green-100'
                        }`}
                        aria-label={`Select model ${model.label}`}
                      >
                        {isSelected ? 'Active' : 'Use'}
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => handleRemoveModel(model.modelId)}
                      disabled={isDownloading}
                      className="text-xs px-2 py-1 rounded text-red-600 hover:bg-red-50 transition-colors disabled:opacity-50"
                      aria-label={`Remove model ${model.label}`}
                    >
                      Remove
                    </button>
                  </div>
                </div>

                {/* Download Progress Bar */}
                {isDownloading && (
                  <div className="mt-2">
                    <div className="w-full bg-gray-200 rounded-full h-1.5">
                      <div
                        className="bg-blue-600 h-1.5 rounded-full transition-all duration-300"
                        style={{
                          width: `${
                            typeof model.status === 'object' && 'downloading' in model.status
                              ? model.status.downloading.progress
                              : 0
                          }%`,
                        }}
                        role="progressbar"
                        aria-valuenow={
                          typeof model.status === 'object' && 'downloading' in model.status
                            ? model.status.downloading.progress
                            : 0
                        }
                        aria-valuemin={0}
                        aria-valuemax={100}
                        aria-label="Download progress"
                      />
                    </div>
                  </div>
                )}

                {/* Error message */}
                {typeof model.status === 'object' && 'error' in model.status && (
                  <p className="text-xs text-red-600 mt-2">
                    {model.status.error}
                  </p>
                )}
              </motion.div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// INSPECTION PANEL SUB-COMPONENT
// ============================================================================

function InspectionPanel({ inspection }: { inspection: InspectionState }) {
  if (inspection.loading) {
    return (
      <div className="bg-white rounded-md p-3 border border-gray-200">
        <div className="flex items-center gap-2">
          <div className="animate-spin h-4 w-4 border-2 border-blue-500 border-t-transparent rounded-full" />
          <span className="text-xs text-gray-600">Inspecting model...</span>
        </div>
      </div>
    );
  }

  if (inspection.error) {
    return (
      <div className="bg-red-50 rounded-md p-3 border border-red-200">
        <p className="text-xs text-red-700">❌ {inspection.error}</p>
      </div>
    );
  }

  if (!inspection.result) return null;

  const { result } = inspection;
  const formatIcon = getFormatIcon(result.format);
  const formatLabel = getFormatLabel(result.format);

  return (
    <div className="bg-white rounded-md p-3 border border-gray-200 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-gray-700">
          {formatIcon} Detected: {formatLabel}
        </span>
        <span className="text-xs text-gray-500">
          {result.totalSizeMb} MB • {result.modelFiles.length} model files
        </span>
      </div>

      {result.isSttModel && (
        <div className="flex items-center gap-1">
          <span className="text-xs text-green-600">✅ Speech-to-text model detected</span>
        </div>
      )}

      {!result.isSttModel && (
        <div className="flex items-center gap-1">
          <span className="text-xs text-yellow-600">
            ⚠️ This may not be a speech-to-text model. Proceed with caution.
          </span>
        </div>
      )}

      {result.format === 'mlx' && (
        <div className="text-xs text-yellow-600 bg-yellow-50 rounded p-2">
          ⚠️ MLX models require macOS with Apple Silicon. On Windows, this model
          will need conversion to ONNX format.
        </div>
      )}

      {result.format === 'safetensors' && (
        <div className="text-xs text-blue-600 bg-blue-50 rounded p-2">
          ℹ️ Safetensors models need conversion to ONNX. Python with transformers
          and torch libraries is required.
        </div>
      )}

      {result.format === 'unknown' && (
        <div className="text-xs text-red-600 bg-red-50 rounded p-2">
          ❌ No recognized model format found. This model may not be compatible.
        </div>
      )}

      {/* File list (collapsed) */}
      <details className="text-xs">
        <summary className="text-gray-500 cursor-pointer hover:text-gray-700">
          View {result.modelFiles.length} model files
        </summary>
        <ul className="mt-1 space-y-0.5 pl-4">
          {result.modelFiles.map((file) => (
            <li key={file} className="text-gray-600 font-mono">
              {file}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

export default CustomModelManager;
