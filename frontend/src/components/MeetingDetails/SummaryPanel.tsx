"use client";

import { Summary, SummaryResponse, SummaryDataResponse, Transcript } from '@/types';
import { EditableTitle } from '@/components/EditableTitle';
import { BlockNoteSummaryView, BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { EmptyStateSummary } from '@/components/EmptyStateSummary';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { SummaryGeneratorButtonGroup } from './SummaryGeneratorButtonGroup';
import { SummaryUpdaterButtonGroup } from './SummaryUpdaterButtonGroup';
import { ChatPanel } from './ChatPanel';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { MessageSquare, FileText } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { RefObject, useState } from 'react';

interface SummaryPanelProps {
  meeting: {
    id: string;
    title: string;
    created_at: string;
  };
  meetingTitle: string;
  onTitleChange: (title: string) => void;
  isEditingTitle: boolean;
  onStartEditTitle: () => void;
  onFinishEditTitle: () => void;
  isTitleDirty: boolean;
  summaryRef: RefObject<BlockNoteSummaryViewRef>;
  isSaving: boolean;
  onSaveAll: () => Promise<void>;
  onCopySummary: () => Promise<void>;
  onOpenFolder: () => Promise<void>;
  aiSummary: Summary | SummaryDataResponse | null;
  summaryStatus: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  onStopGeneration: () => void;
  customPrompt: string;
  summaryResponse: SummaryResponse | null;
  onSaveSummary: (summary: Summary | { markdown?: string; summary_json?: any[] }) => Promise<void>;
  onSummaryChange: (summary: Summary) => void;
  onDirtyChange: (isDirty: boolean) => void;
  summaryError: string | null;
  onRegenerateSummary: () => Promise<void>;
  getSummaryStatusMessage: (status: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error') => string;
  availableTemplates: Array<{ id: string, name: string, description: string }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
  isModelConfigLoading?: boolean;
  onOpenModelSettings?: (openFn: () => void) => void;
}

export function SummaryPanel({
  meeting,
  meetingTitle,
  onTitleChange,
  isEditingTitle,
  onStartEditTitle,
  onFinishEditTitle,
  isTitleDirty,
  summaryRef,
  isSaving,
  onSaveAll,
  onCopySummary,
  onOpenFolder,
  aiSummary,
  summaryStatus,
  transcripts,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onGenerateSummary,
  onStopGeneration,
  customPrompt,
  summaryResponse,
  onSaveSummary,
  onSummaryChange,
  onDirtyChange,
  summaryError,
  onRegenerateSummary,
  getSummaryStatusMessage,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
  isModelConfigLoading = false,
  onOpenModelSettings
}: SummaryPanelProps) {
  const [activeTab, setActiveTab] = useState<'summary' | 'chat'>('summary');
  const { serverAddress } = useSidebar();
  
  const isSummaryLoading = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';
  const isRegenerating = summaryStatus === 'regenerating';

  return (
    <div className="flex-1 min-w-0 flex flex-col bg-white overflow-hidden border-l border-gray-200">
      {/* Tabs */}
      <div className="flex border-b border-gray-200">
        <button
          onClick={() => setActiveTab('summary')}
          className={`flex-1 flex items-center justify-center py-3 text-sm font-medium transition-colors border-b-2 ${
            activeTab === 'summary' 
              ? 'text-blue-600 border-blue-600 bg-blue-50/30' 
              : 'text-gray-500 border-transparent hover:text-gray-700 hover:bg-gray-50'
          }`}
        >
          <FileText size={16} className="mr-2" />
          AI Summary
        </button>
        <button
          onClick={() => setActiveTab('chat')}
          className={`flex-1 flex items-center justify-center py-3 text-sm font-medium transition-colors border-b-2 ${
            activeTab === 'chat' 
              ? 'text-blue-600 border-blue-600 bg-blue-50/30' 
              : 'text-gray-500 border-transparent hover:text-gray-700 hover:bg-gray-50'
          }`}
        >
          <MessageSquare size={16} className="mr-2" />
          Meeting Chat
        </button>
      </div>

      <div className="flex-1 overflow-hidden flex flex-col">
        {activeTab === 'summary' ? (
          <div className="flex-1 flex flex-col overflow-hidden">
            {/* Title area */}
            <div className="p-4 border-b border-gray-200 bg-white shadow-sm z-10">
              {/* Button groups - only show when summary exists */}
              {aiSummary && !isSummaryLoading && (
                <div className="flex items-center justify-center w-full pt-0 gap-2">
                  <div className="flex-shrink-0">
                    <SummaryGeneratorButtonGroup
                      modelConfig={modelConfig}
                      setModelConfig={setModelConfig}
                      onSaveModelConfig={onSaveModelConfig}
                      onGenerateSummary={onGenerateSummary}
                      onStopGeneration={onStopGeneration}
                      customPrompt={customPrompt}
                      summaryStatus={summaryStatus}
                      availableTemplates={availableTemplates}
                      selectedTemplate={selectedTemplate}
                      onTemplateSelect={onTemplateSelect}
                      hasTranscripts={transcripts.length > 0}
                      isModelConfigLoading={isModelConfigLoading}
                      onOpenModelSettings={onOpenModelSettings}
                    />
                  </div>

                  <div className="flex-shrink-0">
                    <SummaryUpdaterButtonGroup
                      isSaving={isSaving}
                      isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
                      onSave={onSaveAll}
                      onCopy={onCopySummary}
                      onFind={() => {
                        console.log('Find in summary clicked');
                      }}
                      onOpenFolder={onOpenFolder}
                      hasSummary={!!aiSummary}
                      onRegenerateSummary={onRegenerateSummary}
                      isRegenerating={isRegenerating}
                    />
                  </div>
                </div>
              )}
            </div>

            {isSummaryLoading ? (
              <div className="flex flex-col h-full bg-gray-50/30">
                <div className="flex items-center justify-center pt-8 pb-4">
                  <SummaryGeneratorButtonGroup
                    modelConfig={modelConfig}
                    setModelConfig={setModelConfig}
                    onSaveModelConfig={onSaveModelConfig}
                    onGenerateSummary={onGenerateSummary}
                    onStopGeneration={onStopGeneration}
                    customPrompt={customPrompt}
                    summaryStatus={summaryStatus}
                    availableTemplates={availableTemplates}
                    selectedTemplate={selectedTemplate}
                    onTemplateSelect={onTemplateSelect}
                    hasTranscripts={transcripts.length > 0}
                    isModelConfigLoading={isModelConfigLoading}
                    onOpenModelSettings={onOpenModelSettings}
                  />
                </div>
                <div className="flex items-center justify-center flex-1">
                  <div className="text-center">
                    <div className="inline-block animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-blue-500 mb-4"></div>
                    <p className="text-gray-600 font-medium">Generating AI Summary...</p>
                  </div>
                </div>
              </div>
            ) : !aiSummary ? (
              <div className="flex flex-col h-full bg-gray-50/30">
                <div className="flex items-center justify-center pt-8 pb-4">
                  <SummaryGeneratorButtonGroup
                    modelConfig={modelConfig}
                    setModelConfig={setModelConfig}
                    onSaveModelConfig={onSaveModelConfig}
                    onGenerateSummary={onGenerateSummary}
                    onStopGeneration={onStopGeneration}
                    customPrompt={customPrompt}
                    summaryStatus={summaryStatus}
                    availableTemplates={availableTemplates}
                    selectedTemplate={selectedTemplate}
                    onTemplateSelect={onTemplateSelect}
                    hasTranscripts={transcripts.length > 0}
                    isModelConfigLoading={isModelConfigLoading}
                    onOpenModelSettings={onOpenModelSettings}
                  />
                </div>
                <EmptyStateSummary
                  onGenerate={() => onGenerateSummary(customPrompt)}
                  hasModel={modelConfig.provider !== null && modelConfig.model !== null}
                  isGenerating={isSummaryLoading}
                />
              </div>
            ) : transcripts?.length > 0 && (
              <div className="flex-1 overflow-y-auto min-h-0 bg-white">
                <div className="p-6 w-full max-w-4xl mx-auto">
                  <BlockNoteSummaryView
                    ref={summaryRef}
                    summaryData={aiSummary}
                    onSave={onSaveSummary}
                    onSummaryChange={onSummaryChange}
                    onDirtyChange={onDirtyChange}
                    status={summaryStatus}
                    error={summaryError}
                    onRegenerateSummary={() => {
                      Analytics.trackButtonClick('regenerate_summary', 'meeting_details');
                      onRegenerateSummary();
                    }}
                    meeting={{
                      id: meeting.id,
                      title: meetingTitle,
                      created_at: meeting.created_at
                    }}
                  />
                </div>
                {summaryStatus !== 'idle' && (
                  <div className="mx-6 mb-6">
                    <div className={`p-4 rounded-xl border ${summaryStatus === 'error' ? 'bg-red-50 border-red-100 text-red-700' :
                      summaryStatus === 'completed' ? 'bg-green-50 border-green-100 text-green-700' :
                        'bg-blue-50 border-blue-100 text-blue-700'
                      }`}>
                      <div className="flex items-center justify-between">
                        <p className="text-sm font-medium">{getSummaryStatusMessage(summaryStatus)}</p>
                        {summaryStatus === 'error' && (
                          <button
                            onClick={() => {
                              Analytics.trackButtonClick('regenerate_summary_from_error', 'meeting_details');
                              onRegenerateSummary();
                            }}
                            className="px-3 py-1.5 text-xs font-semibold bg-red-600 text-white hover:bg-red-700 rounded-lg transition-colors shadow-sm"
                            aria-label="Try regenerating the summary"
                          >
                            Try Again
                          </button>
                        )}
                      </div>
                      {summaryStatus === 'error' && summaryError && (
                        <p className="text-xs mt-2 opacity-80 leading-relaxed">{summaryError}</p>
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>
        ) : (
          <ChatPanel meetingId={meeting.id} serverAddress={serverAddress} />
        )}
      </div>
    </div>
  );
}
