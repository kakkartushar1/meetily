import { isValidBlockNoteBlock, isValidBlockNoteArray, sanitizeBlockNoteArray } from './blocknote-validation';

describe('BlockNote Validation', () => {
  describe('isValidBlockNoteBlock', () => {
    it('should return true for a valid block', () => {
      const block = { id: '1', type: 'paragraph', content: [], children: [] };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should return false for a block with missing type', () => {
      const block = { id: '1', content: [], children: [] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should return false for a block with string content (legacy format)', () => {
      const block = { id: '1', type: 'bullet', content: 'Some text', children: [] };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });

    it('should validate children recursively', () => {
      const block = {
        id: '1',
        type: 'bulletListItem',
        content: [{ type: 'text', text: 'Item' }],
        children: [
          { id: '2', type: 'paragraph', content: [{ type: 'text', text: 'Child' }], children: [] }
        ]
      };
      expect(isValidBlockNoteBlock(block)).toBe(true);
    });

    it('should return false for block with invalid child', () => {
      const block = {
        id: '1',
        type: 'bulletListItem',
        content: [{ type: 'text', text: 'Item' }],
        children: [
          { id: '2', content: [{ type: 'text', text: 'Child' }], children: [] } // missing type
        ]
      };
      expect(isValidBlockNoteBlock(block)).toBe(false);
    });
  });

  describe('isValidBlockNoteArray', () => {
    it('should return true for valid array of blocks', () => {
      const blocks = [
        { id: '1', type: 'heading', content: [{ type: 'text', text: 'Title' }], children: [] },
        { id: '2', type: 'paragraph', content: [{ type: 'text', text: 'Content' }], children: [] },
      ];
      expect(isValidBlockNoteArray(blocks)).toBe(true);
    });

    it('should return false for empty array', () => {
      expect(isValidBlockNoteArray([])).toBe(false);
    });

    it('should return false for array with invalid block', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [], children: [] },
        { id: '2', type: 'bullet', content: 'string', children: [] }, // invalid
      ];
      expect(isValidBlockNoteArray(blocks)).toBe(false);
    });
  });

  describe('sanitizeBlockNoteArray', () => {
    it('should return valid blocks as-is', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [{ type: 'text', text: 'Hello' }], children: [] },
      ];
      const result = sanitizeBlockNoteArray(blocks);
      expect(result.length).toBe(1);
    });

    it('should filter out invalid blocks', () => {
      const blocks = [
        { id: '1', type: 'paragraph', content: [], children: [] },
        { type: 'bullet', content: 'string', children: [] }, // invalid - missing id, has string content
      ];
      const result = sanitizeBlockNoteArray(blocks);
      expect(result.length).toBe(1);
    });

    it('should sanitize nested children', () => {
      const blocks = [
        {
          id: '1',
          type: 'bulletListItem',
          content: [{ type: 'text', text: 'Item' }],
          children: [
            { id: '2', type: 'paragraph', content: [], children: [] }
          ]
        },
      ];
      const result = sanitizeBlockNoteArray(blocks);
      expect(result.length).toBe(1);
      expect(result[0].children?.length).toBe(1);
    });
  });
});