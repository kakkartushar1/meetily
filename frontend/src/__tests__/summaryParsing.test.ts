import { describe, it, expect, vi } from 'vitest';

// ============================================================================
// Test: detectSummaryFormat (from BlockNoteSummaryView)
// ============================================================================

// Re-implement the format detection logic for unit testing
// This mirrors the logic in BlockNoteSummaryView.tsx

function isValidInlineContent(item: any): boolean {
  if (!item || typeof item !== 'object' || Array.isArray(item)) return false;
  if (typeof item.type !== 'string') return false;
  if (item.type === 'text' && typeof item.text !== 'string') return false;
  if (item.styles !== undefined && item.styles !== null) {
    if (typeof item.styles !== 'object' || Array.isArray(item.styles)) return false;
  }
  return true;
}

function isValidBlockNoteBlock(block: any): boolean {
  if (!block || typeof block !== 'object') return false;
  if (typeof block.type !== 'string') return false;
  if (block.content !== undefined && !Array.isArray(block.content) && typeof block.content === 'string') {
    return false;
  }
  // Validate inline content items if content is an array
  if (Array.isArray(block.content)) {
    for (const item of block.content) {
      if (!isValidInlineContent(item)) return false;
    }
  }
  // Validate props if present
  if (block.props !== undefined && block.props !== null) {
    if (typeof block.props !== 'object' || Array.isArray(block.props)) return false;
  }
  return true;
}

function isValidBlockNoteArray(arr: any[]): boolean {
  if (!arr || arr.length === 0) return false;
  // Check ALL blocks to ensure none will cause renderSpec errors
  for (let i = 0; i < arr.length; i++) {
    if (!isValidBlockNoteBlock(arr[i])) return false;
  }
  return true;
}

type SummaryFormat = 'legacy' | 'markdown' | 'blocknote';

function detectSummaryFormat(data: any): { format: SummaryFormat; data: any } {
  if (!data) {
    return { format: 'legacy', data: null };
  }

  if (typeof data === 'string') {
    try {
      data = JSON.parse(data);
    } catch {
      return { format: 'legacy', data: null };
    }
  }

  if (typeof data !== 'object' || Array.isArray(data)) {
    return { format: 'legacy', data: null };
  }

  // Priority 1: BlockNote format
  if (data.summary_json && Array.isArray(data.summary_json)) {
    if (isValidBlockNoteArray(data.summary_json)) {
      return { format: 'blocknote', data };
    }
  }

  // Priority 2: Markdown format
  if (data.markdown && typeof data.markdown === 'string' && data.markdown.trim().length > 0) {
    return { format: 'markdown', data };
  }

  // Priority 3: Legacy JSON
  const hasLegacyStructure = Object.keys(data).some(key => {
    if (key === 'MeetingName' || key === '_section_order' || key === 'markdown' || key === 'summary_json') return false;
    const val = data[key];
    return val && typeof val === 'object' && 'title' in val && 'blocks' in val;
  });

  if (hasLegacyStructure) {
    return { format: 'legacy', data };
  }

  if (data.MeetingName) {
    return { format: 'legacy', data: null };
  }

  return { format: 'legacy', data: null };
}

// ============================================================================
// Test: sanitizeInitialContent (from Editor.tsx)
// ============================================================================

function sanitizeBlock(block: any): any | null {
  if (!block || typeof block !== 'object' || Array.isArray(block)) return null;
  if (typeof block.type !== 'string') return null;
  if (block.content !== undefined && typeof block.content === 'string') return null;

  const sanitized: any = { ...block };
  if (Array.isArray(block.content)) {
    sanitized.content = block.content.filter((item: any) => isValidInlineContent(item));
  }
  if (sanitized.props !== undefined && sanitized.props !== null) {
    if (typeof sanitized.props !== 'object' || Array.isArray(sanitized.props)) {
      sanitized.props = {};
    }
  }
  if (Array.isArray(block.children)) {
    sanitized.children = block.children
      .map((child: any) => sanitizeBlock(child))
      .filter((child: any) => child !== null);
  }
  return sanitized;
}

function sanitizeInitialContent(content: any[] | undefined): any[] | undefined {
  if (!content || !Array.isArray(content) || content.length === 0) {
    return undefined;
  }

  const sanitizedBlocks = content
    .map((block: any) => sanitizeBlock(block))
    .filter((block: any) => block !== null);

  if (sanitizedBlocks.length === 0) {
    return undefined;
  }

  return sanitizedBlocks;
}

// ============================================================================
// Tests
// ============================================================================

describe('Summary Format Detection', () => {
  describe('detectSummaryFormat', () => {
    it('should return legacy null for null input', () => {
      const result = detectSummaryFormat(null);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    it('should return legacy null for undefined input', () => {
      const result = detectSummaryFormat(undefined);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    it('should return legacy null for empty object', () => {
      const result = detectSummaryFormat({});
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    it('should return legacy null for string input that is not JSON', () => {
      const result = detectSummaryFormat('not json');
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    it('should parse JSON string input and detect format', () => {
      const markdownData = JSON.stringify({ markdown: '# Hello World' });
      const result = detectSummaryFormat(markdownData);
      expect(result.format).toBe('markdown');
      expect(result.data.markdown).toBe('# Hello World');
    });

    it('should return legacy null for array input', () => {
      const result = detectSummaryFormat([1, 2, 3]);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    // Markdown format tests
    it('should detect markdown format', () => {
      const data = { markdown: '# Meeting Summary\n\n## Key Points\n- Point 1' };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('markdown');
      expect(result.data).toEqual(data);
    });

    it('should NOT detect markdown format for empty string', () => {
      const data = { markdown: '' };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    it('should NOT detect markdown format for whitespace-only string', () => {
      const data = { markdown: '   \n  \t  ' };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    // BlockNote format tests
    it('should detect valid BlockNote format', () => {
      const data = {
        summary_json: [
          { id: '1', type: 'heading', content: [{ type: 'text', text: 'Title' }], children: [] },
          { id: '2', type: 'paragraph', content: [{ type: 'text', text: 'Content' }], children: [] },
        ],
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('blocknote');
      expect(result.data).toEqual(data);
    });

    it('should NOT detect BlockNote format when content is a string (legacy block)', () => {
      const data = {
        summary_json: [
          { id: '1', type: 'bullet', content: 'This is a string, not an array', color: 'default' },
        ],
      };
      const result = detectSummaryFormat(data);
      // Should fall through to legacy null since no other format matches
      expect(result.format).not.toBe('blocknote');
    });

    it('should NOT detect BlockNote format for empty summary_json array', () => {
      const data = { summary_json: [] };
      const result = detectSummaryFormat(data);
      expect(result.format).not.toBe('blocknote');
    });

    // Legacy format tests
    it('should detect legacy format with sections', () => {
      const data = {
        MeetingName: 'Test Meeting',
        key_points: {
          title: 'Key Points',
          blocks: [
            { id: '1', type: 'bullet', content: 'Point 1', color: 'default' },
          ],
        },
        action_items: {
          title: 'Action Items',
          blocks: [
            { id: '2', type: 'bullet', content: 'Action 1', color: 'default' },
          ],
        },
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('legacy');
      expect(result.data).toEqual(data);
    });

    it('should return legacy null for MeetingName only (no sections)', () => {
      const data = { MeetingName: 'Test Meeting' };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('legacy');
      expect(result.data).toBeNull();
    });

    // Priority tests
    it('should prefer BlockNote over markdown when both exist', () => {
      const data = {
        markdown: '# Hello',
        summary_json: [
          { id: '1', type: 'heading', content: [{ type: 'text', text: 'Title' }], children: [] },
        ],
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('blocknote');
    });

    it('should prefer markdown over legacy when both exist', () => {
      const data = {
        markdown: '# Hello',
        key_points: {
          title: 'Key Points',
          blocks: [{ id: '1', type: 'bullet', content: 'Point 1', color: 'default' }],
        },
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('markdown');
    });

    it('should fall through to markdown when summary_json has invalid blocks', () => {
      const data = {
        markdown: '# Valid Markdown',
        summary_json: [
          { id: '1', type: 'bullet', content: 'string content not array', color: 'default' },
        ],
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('markdown');
    });

    it('should fall through to legacy when summary_json invalid and no markdown', () => {
      const data = {
        summary_json: [
          { id: '1', type: 'bullet', content: 'string content', color: 'default' },
        ],
        key_points: {
          title: 'Key Points',
          blocks: [{ id: '1', type: 'bullet', content: 'Point 1', color: 'default' }],
        },
      };
      const result = detectSummaryFormat(data);
      expect(result.format).toBe('legacy');
      expect(result.data).toEqual(data);
    });
  });
});

describe('BlockNote Content Validation', () => {
  describe('isValidBlockNoteBlock', () => {
    it('should validate a proper BlockNote block', () => {
      const block = { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Hello' }], children: [] };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should validate a block with undefined content', () => {
      const block = { id: '1', type: 'paragraph', children: [] };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should reject a block with string content (legacy format)', () => {
      const block = { id: '1', type: 'bullet', content: 'This is legacy', color: 'default' };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject null', () => {
      expect(isValidBlockNoteBlock(null)).toBe(false);
    });

    it('should reject non-object', () => {
      expect(isValidBlockNoteBlock('string')).toBe(false);
    });

    it('should reject block without type', () => {
      const block = { id: '1', content: [] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject block with non-string type', () => {
      const block = { id: '1', type: 123, content: [] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject block with invalid inline content items', () => {
      // This is the key case that causes renderSpec errors
      const block = { id: '1', type: 'paragraph', content: [{ type: 'text' }] }; // missing 'text' property
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject block with non-object inline content items', () => {
      const block = { id: '1', type: 'paragraph', content: ['plain string'] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject block with array inline content items', () => {
      const block = { id: '1', type: 'paragraph', content: [[1, 2, 3]] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should reject block with inline content missing type', () => {
      const block = { id: '1', type: 'paragraph', content: [{ text: 'hello' }] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should validate block with hardBreak inline content', () => {
      const block = { id: '1', type: 'paragraph', content: [{ type: 'hardBreak' }] };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should validate block with styled text inline content', () => {
      const block = { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Bold', styles: { bold: true } }] };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should reject block with invalid styles (array instead of object)', () => {
      const block = { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Hello', styles: ['bold'] }] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });
  });

  describe('isValidBlockNoteArray', () => {
    it('should validate array of valid blocks', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Hello' }], children: [] },
        { id: '2', type: 'heading', content: [{ type: 'text', text: 'Title' }], children: [] },
      ];
      expect(isValidBlockNoteArray(blocks)).toBe(true);
    });

    it('should reject empty array', () => {
      expect(isValidBlockNoteArray([])).toBe(false);
    });

    it('should reject array with legacy blocks', () => {
      const blocks = [
        { id: '1', type: 'bullet', content: 'Legacy string content', color: 'default' },
      ];
      expect(isValidBlockNoteArray(blocks)).toBe(false);
    });

    it('should reject null', () => {
      expect(isValidBlockNoteArray(null as any)).toBe(false);
    });

    it('should reject array where ANY block has invalid inline content', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Valid' }], children: [] },
        { id: '2', type: 'paragraph', content: [{ type: 'text' }], children: [] }, // invalid: missing text
      ];
      expect(isValidBlockNoteArray(blocks)).toBe(false);
    });
  });
});

describe('Editor Content Sanitization', () => {
  describe('sanitizeInitialContent', () => {
    it('should return undefined for undefined input', () => {
      expect(sanitizeInitialContent(undefined)).toBeUndefined();
    });

    it('should return undefined for empty array', () => {
      expect(sanitizeInitialContent([])).toBeUndefined();
    });

    it('should return undefined for null input', () => {
      expect(sanitizeInitialContent(null as any)).toBeUndefined();
    });

    it('should return valid BlockNote blocks as-is', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Hello' }], children: [] },
      ];
      expect(sanitizeInitialContent(blocks)).toEqual(blocks);
    });

    it('should return undefined for legacy blocks with string content', () => {
      const blocks = [
        { id: '1', type: 'bullet', content: 'Legacy string', color: 'default' },
      ];
      expect(sanitizeInitialContent(blocks)).toBeUndefined();
    });

    it('should filter out invalid blocks and keep valid ones from mixed input', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Valid' }] },
        { id: '2', type: 'bullet', content: 'Invalid string content' },
      ];
      const result = sanitizeInitialContent(blocks);
      // New behavior: sanitize filters out invalid blocks instead of rejecting all
      expect(result).toBeDefined();
      expect(result).toHaveLength(1);
      expect(result![0].type).toBe('paragraph');
    });

    it('should return undefined for blocks without type', () => {
      const blocks = [
        { id: '1', content: [{ type: 'text', text: 'No type' }] },
      ];
      expect(sanitizeInitialContent(blocks)).toBeUndefined();
    });
  });
});

describe('Summary Data Flow Integration', () => {
  it('should handle the new backend format (markdown only)', () => {
    // This is what the Rust backend stores: { "markdown": "..." }
    const backendData = { markdown: '## Key Points\n- Point 1\n- Point 2' };
    const result = detectSummaryFormat(backendData);
    expect(result.format).toBe('markdown');
    expect(result.data.markdown).toBe('## Key Points\n- Point 1\n- Point 2');
  });

  it('should handle saved BlockNote format (after user edits)', () => {
    // After user edits in BlockNote, the save includes both markdown and summary_json
    const savedData = {
      markdown: '## Key Points\n- Point 1',
      summary_json: [
        { id: '1', type: 'heading', props: { level: 2 }, content: [{ type: 'text', text: 'Key Points' }], children: [] },
        { id: '2', type: 'bulletListItem', content: [{ type: 'text', text: 'Point 1' }], children: [] },
      ],
    };
    const result = detectSummaryFormat(savedData);
    // BlockNote format should take priority
    expect(result.format).toBe('blocknote');
  });

  it('should handle old legacy format from Python backend', () => {
    // Old format from the Python backend
    const legacyData = {
      MeetingName: 'Team Standup',
      _section_order: ['key_points', 'action_items'],
      key_points: {
        title: 'Key Points',
        blocks: [
          { id: 'kp-1', type: 'bullet', content: 'Discussed project timeline', color: 'default' },
          { id: 'kp-2', type: 'bullet', content: 'Budget approved', color: 'default' },
        ],
      },
      action_items: {
        title: 'Action Items',
        blocks: [
          { id: 'ai-1', type: 'bullet', content: 'Follow up with client', color: 'default' },
        ],
      },
    };
    const result = detectSummaryFormat(legacyData);
    expect(result.format).toBe('legacy');
    expect(result.data).toEqual(legacyData);
  });

  it('should handle double-encoded JSON string from backend', () => {
    const innerData = { markdown: '# Summary\n## Points\n- Point 1' };
    const doubleEncoded = JSON.stringify(innerData);
    const result = detectSummaryFormat(doubleEncoded);
    expect(result.format).toBe('markdown');
    expect(result.data.markdown).toBe('# Summary\n## Points\n- Point 1');
  });

  it('should handle corrupted/malformed data gracefully', () => {
    const corruptedData = { some_random_key: 'value', another: 42 };
    const result = detectSummaryFormat(corruptedData);
    expect(result.format).toBe('legacy');
    expect(result.data).toBeNull();
  });

  it('should handle data with only MeetingName (no sections)', () => {
    const data = { MeetingName: 'Test' };
    const result = detectSummaryFormat(data);
    expect(result.format).toBe('legacy');
    expect(result.data).toBeNull();
  });
});
