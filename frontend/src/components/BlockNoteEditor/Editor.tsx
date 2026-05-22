"use client";

import { useEffect, useMemo, useState } from "react";
import { PartialBlock, Block } from "@blocknote/core";
import "@blocknote/shadcn/style.css";
import "@blocknote/core/fonts/inter.css";
import { useCreateBlockNote } from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import { sanitizeInitialContent } from "../../lib/blocknote-validation";
import EditorErrorBoundary from "./EditorErrorBoundary";

interface EditorProps {
  initialContent?: Block[];
  onChange?: (blocks: Block[]) => void;
  editable?: boolean;
}

export default function Editor({ initialContent, onChange, editable = true }: EditorProps) {
  const [hasError, setHasError] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string>('');

  const safeInitialContent = useMemo(
    () => sanitizeInitialContent(initialContent),
    [initialContent]
  );

  console.log('📝 EDITOR: Initializing BlockNote editor with blocks:', {
    hasContent: !!initialContent,
    blocksCount: initialContent?.length || 0,
    sanitizedCount: safeInitialContent?.length || 0,
    wasSanitized: !!initialContent && !safeInitialContent,
    editable
  });

  let editor: any = null;
  try {
    editor = useCreateBlockNote({
      initialContent: safeInitialContent as PartialBlock[],
    });
  } catch (error) {
    console.error('❌ EDITOR: Failed to create BlockNote editor:', error);
    if (!hasError) {
      setHasError(true);
      setErrorMessage(error instanceof Error ? error.message : 'Unknown editor initialization error');
    }
  }

  if (editor) {
    console.log('📝 EDITOR: BlockNote editor created successfully, document blocks:', editor.document?.length);
  }

  useEffect(() => {
    if (!onChange || !editor) return;

    const handleChange = () => {
      onChange(editor.document);
    };

    const unsubscribe = editor.onChange(handleChange);

    return () => {
      if (typeof unsubscribe === 'function') {
        unsubscribe();
      }
    };
  }, [editor, onChange]);

  const handleRenderError = (error: Error) => {
    console.error('❌ EDITOR: Render error caught:', error.message);
    setHasError(true);
    setErrorMessage(error.message);
  };

  if (hasError || !editor) {
    return (
      <div className="p-4 bg-yellow-50 border border-yellow-200 rounded-lg">
        <p className="text-yellow-700 text-sm">
          Unable to render summary in editor. The summary data format may be incompatible.
        </p>
        {errorMessage && (
          <p className="text-yellow-600 text-xs mt-1 font-mono">
            Error: {errorMessage}
          </p>
        )}
        <p className="text-yellow-600 text-xs mt-1">
          Try regenerating the summary to fix this issue.
        </p>
      </div>
    );
  }

  return (
    <EditorErrorBoundary onError={handleRenderError}>
      <BlockNoteView editor={editor} editable={editable} theme="light" />
    </EditorErrorBoundary>
  );
}
