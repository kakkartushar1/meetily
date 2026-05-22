'use client';

import { useEffect, useState } from 'react';

/**
 * Checks if an error is related to BlockNote/summary rendering.
 * These errors should show a more specific message and allow retry.
 */
function isSummaryRenderError(error: Error): boolean {
  const message = error.message || '';
  const stack = error.stack || '';
  return (
    message.includes('renderSpec') ||
    message.includes('Invalid array') ||
    message.includes('BlockNote') ||
    message.includes('initialContent') ||
    message.includes('blocknote') ||
    stack.includes('renderSpec') ||
    stack.includes('BlockNote') ||
    stack.includes('blocknote')
  );
}

/**
 * Error boundary for the meeting-details route.
 * Catches errors from meeting data loading, summary generation,
 * and transcript pagination that would otherwise crash the entire app.
 */
export default function MeetingDetailsError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const isSummaryError = isSummaryRenderError(error);
  const [showDetails, setShowDetails] = useState(false);

  useEffect(() => {
    console.error('[Meeting Details Error Boundary] Caught error:', error);
    console.error('[Meeting Details Error Boundary] Error message:', error.message);
    console.error('[Meeting Details Error Boundary] Error stack:', error.stack);
    if (isSummaryError) {
      console.warn('[Meeting Details Error Boundary] This appears to be a summary rendering error (Invalid array/renderSpec). The summary data format may be incompatible with the BlockNote editor.');
    }
  }, [error, isSummaryError]);

  return (
    <div className="flex items-center justify-center h-screen bg-gray-50">
      <div className="text-center max-w-lg mx-auto p-8">
        <div className="mb-6">
          <svg
            className={`mx-auto h-16 w-16 ${isSummaryError ? 'text-orange-400' : 'text-red-400'}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.5}
              d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z"
            />
          </svg>
        </div>
        <h2 className="text-xl font-semibold text-gray-900 mb-2">
          {isSummaryError ? 'Summary display error' : 'Failed to load meeting'}
        </h2>
        <p className="text-gray-600 mb-4 text-sm">
          {isSummaryError
            ? 'The meeting summary data format is incompatible with the editor. This can happen when the summary was generated with a different version.'
            : (error.message || 'There was a problem loading the meeting details. This may be due to a connection issue or missing data.')
          }
        </p>
        {isSummaryError && (
          <p className="text-gray-500 mb-4 text-xs">
            Click "Retry" to reload the page. If the issue persists, try regenerating the summary from the meeting details page.
          </p>
        )}
        <div className="flex gap-3 justify-center mb-4">
          <button
            onClick={reset}
            className="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors text-sm font-medium"
          >
            Retry
          </button>
          <button
            onClick={() => {
              if (typeof window !== 'undefined') {
                window.location.href = '/';
              }
            }}
            className="px-4 py-2 bg-gray-200 text-gray-700 rounded-lg hover:bg-gray-300 transition-colors text-sm font-medium"
          >
            Back to Home
          </button>
        </div>
        <button
          onClick={() => setShowDetails(!showDetails)}
          className="text-xs text-gray-400 hover:text-gray-600 underline"
        >
          {showDetails ? 'Hide' : 'Show'} error details
        </button>
        {showDetails && (
          <div className="mt-3 p-3 bg-gray-100 rounded-lg text-left">
            <p className="text-xs text-gray-600 font-mono break-all">
              {error.message}
            </p>
            {error.stack && (
              <pre className="text-xs text-gray-500 mt-2 overflow-auto max-h-40 whitespace-pre-wrap">
                {error.stack.split('\n').slice(0, 5).join('\n')}
              </pre>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
