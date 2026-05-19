/**
 * Transcription Model Catalog (TypeScript mirror of Rust model_catalog.rs)
 *
 * Single source of truth for frontend model display metadata.
 * Runtime is resolved from this catalog by model name.
 *
 * IMPORTANT: Keep in sync with Rust catalog in src-tauri/src/model_catalog.rs
 */

// ============================================================================
// RUNTIME & TYPES
// ============================================================================

/** Runtime backend used to execute a transcription model. */
export type ModelRuntime = 'localWhisper' | 'parakeet' | 'nemo';

/** Detected format of a model from HuggingFace or local path. */
export type ModelFormat = 'onnx' | 'mlx' | 'safetensors' | 'nemoCheckpoint' | 'unknown';

/** Status of a custom model download/conversion. */
export type CustomModelStatus =
  | 'pending'
  | { downloading: { progress: number } }
  | 'converting'
  | 'ready'
  | { error: string };

/** A single entry in the transcription model catalog. */
export interface CatalogEntry {
  /** Unique model identifier used in DB `model` column. */
  modelId: string;
  /** Provider string stored in DB `provider` column. */
  provider: 'whisper' | 'localWhisper' | 'parakeet';
  /** Runtime backend that executes this model. */
  runtime: ModelRuntime;
  /** HuggingFace repo ID (for downloadable models). */
  repo: string | null;
  /** Primary model filename within the repo or local directory. */
  file: string | null;
  /** Approximate download size in MB. */
  sizeMb: number;
  /** Expected audio sample rate in Hz. */
  sampleRate: number;
  /** Human-readable label for UI display. */
  label: string;
  /** Short description for UI display. */
  description: string;
  /** Emoji icon for display. */
  icon: string;
}

/** A user-added custom model from HuggingFace or local path. */
export interface CustomModelCatalogEntry {
  /** Unique model identifier (e.g., "custom/nvidia/parakeet-tdt-0.6b"). */
  modelId: string;
  /** Provider string. */
  provider: string;
  /** Runtime backend. */
  runtime: ModelRuntime;
  /** Detected model format. */
  format: ModelFormat;
  /** HuggingFace repo ID (if sourced from HuggingFace). */
  repo: string | null;
  /** Local file path (if sourced from local filesystem). */
  localPath: string | null;
  /** List of model files. */
  files: string[];
  /** Approximate download size in MB. */
  sizeMb: number;
  /** Expected audio sample rate in Hz. */
  sampleRate: number;
  /** Human-readable label for UI display. */
  label: string;
  /** Short description for UI display. */
  description: string;
  /** Current status of the custom model. */
  status: CustomModelStatus;
}

/** Result of inspecting a HuggingFace model. */
export interface ModelInspectionResult {
  modelId: string;
  files: Array<{ filename: string; size: number }>;
  format: ModelFormat;
  totalSizeMb: number;
  modelFiles: string[];
  isSttModel: boolean;
}

// ============================================================================
// PARAKEET MODEL CATALOG
// ============================================================================

/** Complete catalog of Parakeet-family models (ONNX + NeMo). */
export const PARAKEET_MODEL_CATALOG: readonly CatalogEntry[] = [
  // ── ONNX Parakeet models (existing) ──────────────────────────────────
  {
    modelId: 'parakeet-tdt-0.6b-v3-int8',
    provider: 'parakeet',
    runtime: 'parakeet',
    repo: null,
    file: null,
    sizeMb: 670,
    sampleRate: 16000,
    label: 'Parakeet TDT 0.6B v3 (Int8)',
    description: 'Ultra Fast – real time on M4 Max, latest version with int8 quantization',
    icon: '⚡',
  },
  {
    modelId: 'parakeet-tdt-0.6b-v2-int8',
    provider: 'parakeet',
    runtime: 'parakeet',
    repo: null,
    file: null,
    sizeMb: 661,
    sampleRate: 16000,
    label: 'Parakeet TDT 0.6B v2 (Int8)',
    description: 'Fast – previous version with int8 quantization, good balance of speed and accuracy',
    icon: '⚡',
  },
  // ── NeMo Parakeet models (new) ───────────────────────────────────────
  {
    modelId: 'nvidia/parakeet-rnnt-1.1b',
    provider: 'parakeet',
    runtime: 'nemo',
    repo: 'nvidia/parakeet-rnnt-1.1b',
    file: 'parakeet-rnnt-1.1b.nemo',
    sizeMb: 4280,
    sampleRate: 16000,
    label: 'Parakeet RNNT 1.1B',
    description: 'High-accuracy English ASR – opt-in download, requires ~4.3 GB',
    icon: '🎯',
  },
] as const;

// ============================================================================
// LOOKUP HELPERS
// ============================================================================

/** Look up a catalog entry by modelId. */
export function lookupModel(modelId: string): CatalogEntry | undefined {
  return PARAKEET_MODEL_CATALOG.find((e) => e.modelId === modelId);
}

/** Resolve the runtime for a given (provider, modelId) pair. */
export function resolveRuntime(provider: string, modelId: string): ModelRuntime | undefined {
  if (provider === 'localWhisper' || provider === 'whisper') {
    return 'localWhisper';
  }
  return lookupModel(modelId)?.runtime;
}

/** Check whether a modelId refers to a NeMo runtime model. */
export function isNemoModel(modelId: string): boolean {
  return resolveRuntime('parakeet', modelId) === 'nemo';
}

/** Get all catalog entries for a specific runtime. */
export function modelsForRuntime(runtime: ModelRuntime): CatalogEntry[] {
  return PARAKEET_MODEL_CATALOG.filter((e) => e.runtime === runtime);
}

/** Get display info for a model by its ID (used by ParakeetModelManager). */
export function getModelDisplayInfo(modelId: string): {
  friendlyName: string;
  icon: string;
  description: string;
  sizeMb: number;
  runtime: ModelRuntime;
} | undefined {
  const entry = lookupModel(modelId);
  if (!entry) return undefined;
  return {
    friendlyName: entry.label,
    icon: entry.icon,
    description: entry.description,
    sizeMb: entry.sizeMb,
    runtime: entry.runtime,
  };
}

/** Check whether a modelId refers to a custom (user-added) model. */
export function isCustomModel(modelId: string): boolean {
  return modelId.startsWith('custom/') || modelId.startsWith('local/');
}

/** Get a human-readable label for a model format. */
export function getFormatLabel(format: ModelFormat): string {
  switch (format) {
    case 'onnx': return 'ONNX';
    case 'mlx': return 'MLX';
    case 'safetensors': return 'Safetensors';
    case 'nemoCheckpoint': return 'NeMo';
    case 'unknown': return 'Unknown';
    default: return format;
  }
}

/** Get an icon for a model format. */
export function getFormatIcon(format: ModelFormat): string {
  switch (format) {
    case 'onnx': return '⚡';
    case 'mlx': return '🍎';
    case 'safetensors': return '🔒';
    case 'nemoCheckpoint': return '🎯';
    case 'unknown': return '❓';
    default: return '📦';
  }
}

/** Get the friendly display name for a model (built-in or custom). */
export function getModelDisplayName(modelId: string): string {
  const entry = lookupModel(modelId);
  if (entry) return entry.label;
  // For custom models, extract a readable name from the ID
  if (modelId.startsWith('custom/')) return modelId.replace('custom/', '');
  if (modelId.startsWith('local/')) return modelId.replace('local/', '');
  return modelId;
}
