"use client";

import { useState, useEffect, useCallback, useRef, forwardRef, useImperativeHandle, Component } from 'react';
import dynamic from 'next/dynamic';
import { Summary, SummaryDataResponse, BlockNoteBlock } from '@/types';
import { AISummary } from './index';
import { Block } from '@blocknote/core';
import { useCreateBlockNote } from '@blocknote/react';
import { BlockNoteView } from '@blocknote/shadcn';
import "@blocknote/shadcn/style.css";

// Import unified validation utility
import {
  isValidInlineContent,
  isValidBlockNoteBlock,
  isValidBlockNoteArray,
  sanitizeBlockNoteArray,
  detectSummaryFormat,
} from '@/lib/blocknote-validation';

// Dynamically import BlockNote Editor to avoid SSR issues
const Editor = dynamic(() => import('../BlockNoteEditor/Editor'), { ssr: false });

interface BlockNoteSummaryViewProps {
  summaryData: SummaryDataResponse | Summary | null;
  onSave?: (data: { markdown?: string; summary_json?: BlockNoteBlock[] }) => void;
  onSummaryChange?: (summary: Summary) => void;
  status?: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
  error?: string | null;
  onRegenerateSummary?: () => void;
  meeting?: {
    id: string;
    title: string;
    created_at: string;
  };
  onDirtyChange?: (isDirty: boolean) => void;
}

export interface BlockNoteSummaryViewRef {
  saveSummary: () => Promise<void>;
  getMarkdown: () => Promise<string>;
  isDirty: boolean;
}

export const BlockNoteSummaryView = forwardRef<BlockNoteSummaryViewRef, BlockNoteSummaryViewProps>(({
  summaryData,
  onSave,
  onSummaryChange,
  status = 'idle',
  error = null,
  onRegenerateSummary,
  meeting,
  onDirtyChange
}, ref) => {
  const [renderError, setRenderError] = useState<string | null>(null);
  const { format, data } = detectSummaryFormat(summaryData);

  // ─── Diagnostic logging (Task 2) ────────────────────────────────────────
  // These logs confirm the format detected and the validation results so we
  // can identify missing `children` validation as the root cause.
  console.log(
    '🔍 BlockNoteSummaryView: detected format =', format,
    '| summaryData type =', typeof summaryData,
    '| keys =', summaryData ? Object.keys(summaryData) : 'null',
  );

  if (format === 'blocknote' && data?.summary_json) {
    const rawBlocks = data.summary_json;
    const validationResult = isValidBlockNoteArray(rawBlocks);
    console.log(
      '🔍 BlockNoteSummaryView: summary_json block count =', rawBlocks.length,
      '| isValidBlockNoteArray (with children) =', validationResult,
    );

    // Log the first block in detail to surface any malformed children
    if (rawBlocks.length > 0) {
      console.log(
        '🔍 BlockNoteSummaryView: first block sample =',
        JSON.stringify(rawBlocks[0], null, 2),
      );
    }

    if (!validationResult) {
      console.warn(
        '⚠️ BlockNoteSummaryView: summary_json FAILED validation – sanitiseBlockNoteArray will be applied before render.',
      );
    }
  }
  // ─────────────────────────────────────────────────────────────────────────
  const [isDirty, setIsDirty] = useState(false);
  const [currentBlocks, setCurrentBlocks] = useState<Block[]>([]);
  const [isSaving, setIsSaving] = useState(false);
  const isContentLoaded = useRef(false);

  // Create BlockNote editor for markdown parsing
  const editor = useCreateBlockNote({
    initialContent: undefined
  });

  // Parse markdown to blocks when format is markdown
  useEffect(() => {
    if (format === 'markdown' && data?.markdown && editor) {
      const markdownContent: string = data.markdown;
      const loadMarkdown = async () => {
        try {
          console.log('📝 Parsing markdown to BlockNote blocks...');
          const blocks = await editor.tryParseMarkdownToBlocks(markdownContent);
          editor.replaceBlocks(editor.document, blocks);
          console.log('✅ Markdown parsed successfully');

          // Delay to ensure editor has finished rendering before allowing onChange
          setTimeout(() => {
            isContentLoaded.current = true;
          }, 100);
        } catch (err) {
          console.error('❌ Failed to parse markdown:', err);
        }
      };
      loadMarkdown();
    }
  }, [format, data?.markdown, editor]);

  // Set content loaded flag for blocknote format
  useEffect(() => {
    if (format === 'blocknote' && data?.summary_json) {
      // Delay to ensure editor has finished rendering
      setTimeout(() => {
        isContentLoaded.current = true;
      }, 100);
    }
  }, [format, data?.summary_json]);

  const handleEditorChange = useCallback((blocks: Block[]) => {
    // Only set dirty flag if content has finished loading
    if (isContentLoaded.current) {
      setCurrentBlocks(blocks);
      setIsDirty(true);
    }
  }, []);

  // Notify parent of dirty state changes
  useEffect(() => {
    if (onDirtyChange) {
      onDirtyChange(isDirty);
    }
  }, [isDirty, onDirtyChange]);

  const handleSave = useCallback(async () => {
    if (!onSave || !isDirty) return;

    setIsSaving(true);
    try {
      console.log('💾 Saving BlockNote content...');

      // Generate markdown from current blocks
      const markdown = await editor.blocksToMarkdownLossy(currentBlocks);

      onSave({
        markdown: markdown,
        summary_json: currentBlocks as unknown as BlockNoteBlock[]
      });

      setIsDirty(false);
      console.log('✅ Save successful');
    } catch (err) {
      console.error('❌ Save failed:', err);
      alert('Failed to save changes. Please try again.');
    } finally {
      setIsSaving(false);
    }
  }, [onSave, isDirty, currentBlocks, editor]);

  // Expose methods to parent via ref
  useImperativeHandle(ref, () => ({
    saveSummary: handleSave,
    getMarkdown: async () => {
      try {
        console.log('🔍 getMarkdown called, format:', format);
        console.log('🔍 currentBlocks length:', currentBlocks.length);
        console.log('🔍 data:', data);

        // For markdown format - use the main editor
        if (format === 'markdown' && editor) {
          console.log('📝 Using markdown editor, blocks:', editor.document.length);
          const markdown = await editor.blocksToMarkdownLossy(editor.document);
          console.log('📝 Generated markdown length:', markdown.length);
          return markdown;
        }

        // For blocknote format - use currentBlocks state
        if (format === 'blocknote') {
          console.log('📝 BlockNote format, currentBlocks:', currentBlocks.length);
          if (currentBlocks.length > 0 && editor) {
            const markdown = await editor.blocksToMarkdownLossy(currentBlocks);
            console.log('📝 Generated markdown from blocks, length:', markdown.length);
            return markdown;
          }
          // Fallback: if we have the original data with markdown
          if (data?.markdown) {
            console.log('📝 Using fallback markdown from data');
            return data.markdown;
          }
        }

        // For legacy format - return empty (handled by parent)
        console.warn('⚠️ Cannot generate markdown for legacy format, returning empty');
        return '';
      } catch (err) {
        console.error('❌ Failed to generate markdown:', err);
        return '';
      }
    },
    isDirty
  }), [handleSave, isDirty, editor, format, currentBlocks, data]);

  // Render legacy format
  if (format === 'legacy') {
    console.log('🎨 Rendering LEGACY format');
    return (
      <AISummary
        summary={summaryData as Summary}
        status={status}
        error={error}
        onSummaryChange={onSummaryChange || (() => { })}
        onRegenerateSummary={onRegenerateSummary || (() => { })}
        meeting={meeting}
      />
    );
  }

  // Show render error fallback
  if (renderError) {
    console.error('🎨 BlockNoteSummaryView render error:', renderError);
    return (
      <div className="p-4 bg-yellow-50 border border-yellow-200 rounded-lg">
        <p className="text-yellow-700 text-sm font-medium">Unable to display summary</p>
        <p className="text-yellow-600 text-xs mt-1">{renderError}</p>
        <p className="text-yellow-600 text-xs mt-1">Try regenerating the summary to fix this issue.</p>
        {onRegenerateSummary && (
          <button
            onClick={() => {
              setRenderError(null);
              onRegenerateSummary();
            }}
            className="mt-2 px-3 py-1 text-xs bg-yellow-100 hover:bg-yellow-200 text-yellow-800 rounded border border-yellow-300"
          >
            Regenerate Summary
          </button>
        )}
      </div>
    );
  }

  // Render BlockNote format (has summary_json)
  if (format === 'blocknote') {
    console.log('🎨 Rendering BLOCKNOTE format (direct), blocks:', data?.summary_json?.length);
    // Extra safety: validate summary_json one more time before passing to Editor
    if (!data?.summary_json || !Array.isArray(data.summary_json) || data.summary_json.length === 0) {
      console.error('❌ BLOCKNOTE format but summary_json is empty/invalid, falling back to legacy');
      return (
        <AISummary
          summary={summaryData as Summary}
          status={status}
          error={error}
          onSummaryChange={onSummaryChange || (() => { })}
          onRegenerateSummary={onRegenerateSummary || (() => { })}
          meeting={meeting}
        />
      );
    }

    // Sanitize the blocks to prevent ProseMirror renderSpec errors
    // This handles cases where saved blocks have malformed inline content
    const sanitizedBlocks = sanitizeBlockNoteArray(data.summary_json);
    if (sanitizedBlocks.length === 0) {
      console.error('❌ BLOCKNOTE format but all blocks were invalid after sanitization, falling back to markdown if available');
      // Try to fall through to markdown rendering if markdown exists
      if (data?.markdown && typeof data.markdown === 'string' && data.markdown.trim().length > 0) {
        console.log('🔄 Falling back to markdown rendering');
        // Don't return here - let it fall through to the markdown section below
      } else {
        return (
          <AISummary
            summary={summaryData as Summary}
            status={status}
            error={error}
            onSummaryChange={onSummaryChange || (() => { })}
            onRegenerateSummary={onRegenerateSummary || (() => { })}
            meeting={meeting}
          />
        );
      }
    }

    if (sanitizedBlocks.length > 0) {
      if (sanitizedBlocks.length !== data.summary_json.length) {
        console.warn(`⚠️ Sanitized ${data.summary_json.length - sanitizedBlocks.length} invalid blocks from summary_json`);
      }
      return (
        <div className="flex flex-col w-full">
          <div className="w-full">
            <Editor
              initialContent={sanitizedBlocks as unknown as Block[]}
              onChange={(blocks) => {
                console.log('📝 Editor blocks changed:', blocks.length);
                handleEditorChange(blocks);
              }}
              editable={true}
            />
          </div>
        </div>
      );
    }
  }

  // Render Markdown format (parse and display in BlockNote)
  if (format === 'markdown') {
    console.log('🎨 Rendering MARKDOWN format (parsed to BlockNote), markdown length:', data?.markdown?.length);
    // Safety: ensure we have valid markdown content
    if (!data?.markdown || typeof data.markdown !== 'string' || data.markdown.trim().length === 0) {
      console.error('❌ MARKDOWN format but markdown content is empty/invalid');
      return (
        <div className="p-4 bg-yellow-50 border border-yellow-200 rounded-lg">
          <p className="text-yellow-700 text-sm font-medium">No summary content available</p>
          <p className="text-yellow-600 text-xs mt-1">The summary appears to be empty. Try regenerating it.</p>
          {onRegenerateSummary && (
            <button
              onClick={onRegenerateSummary}
              className="mt-2 px-3 py-1 text-xs bg-yellow-100 hover:bg-yellow-200 text-yellow-800 rounded border border-yellow-300"
            >
              Regenerate Summary
            </button>
          )}
        </div>
      );
    }
    return (
      <div className="flex flex-col w-full">
        <div className="w-full">
          <BlockNoteView
            editor={editor}
            editable={true}
            onChange={() => {
              if (isContentLoaded.current) {
                handleEditorChange(editor.document);
              }
            }}
            theme="light"
          />
        </div>
      </div>
    );
  }

  return null;
});

BlockNoteSummaryView.displayName = 'BlockNoteSummaryView';
