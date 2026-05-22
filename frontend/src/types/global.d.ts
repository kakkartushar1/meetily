/**
 * Global type declarations for the Meetily frontend.
 */

// Extend the Window interface to include Tauri internals
declare global {
  interface Window {
    /** Tauri internal APIs - available when running inside Tauri webview */
    __TAURI_INTERNALS__?: Record<string, unknown>;
    /** Recording stop handler exposed by useRecordingStop hook */
    handleRecordingStop?: (callApi: boolean) => void;
  }
}

export {};
