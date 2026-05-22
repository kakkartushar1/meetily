"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Save, Loader2, RefreshCw } from 'lucide-react';
import Analytics from '@/lib/analytics';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  onFind?: () => void;
  onOpenFolder: () => Promise<void>;
  hasSummary: boolean;
  onRegenerateSummary?: () => Promise<void>;
  isRegenerating?: boolean;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  onFind,
  onOpenFolder,
  hasSummary,
  onRegenerateSummary,
  isRegenerating = false,
}: SummaryUpdaterButtonGroupProps) {
  return (
    <ButtonGroup>
      {/* Save button */}
      <Button
        variant="outline"
        size="sm"
        className={`${isDirty ? 'bg-green-200' : ""}`}
        title={isSaving ? "Saving" : "Save Changes"}
        onClick={() => {
          Analytics.trackButtonClick('save_changes', 'meeting_details');
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="animate-spin" />
            <span className="hidden lg:inline">Saving...</span>
          </>
        ) : (
          <>
            <Save />
            <span className="hidden lg:inline">Save</span>
          </>
        )}
      </Button>

      {/* Copy button */}
      <Button
        variant="outline"
        size="sm"
        title="Copy Summary"
        onClick={() => {
          Analytics.trackButtonClick('copy_summary', 'meeting_details');
          onCopy();
        }}
        disabled={!hasSummary}
        className="cursor-pointer"
      >
        <Copy />
        <span className="hidden lg:inline">Copy</span>
      </Button>

      {/* Regenerate Summary button */}
      {onRegenerateSummary && (
        <Button
          variant="outline"
          size="sm"
          title="Regenerate Summary"
          onClick={() => {
            Analytics.trackButtonClick('regenerate_summary', 'meeting_details');
            onRegenerateSummary();
          }}
          disabled={isRegenerating || !hasSummary}
          className="cursor-pointer bg-gradient-to-r from-amber-50 to-orange-50 hover:from-amber-100 hover:to-orange-100 border-amber-200"
          aria-label="Regenerate the meeting summary"
        >
          {isRegenerating ? (
            <>
              <Loader2 className="animate-spin" />
              <span className="hidden lg:inline">Regenerating...</span>
            </>
          ) : (
            <>
              <RefreshCw />
              <span className="hidden lg:inline">Regenerate</span>
            </>
          )}
        </Button>
      )}
    </ButtonGroup>
  );
}
