
// frontend/src/lib/blocknote-validation.ts

/**
 * Checks if an item is a valid BlockNote inline content element.
 * This is a stricter check than the type definitions to prevent runtime errors.
 */
export function isValidInlineContent(item: any): boolean {
  if (!item || typeof item !== 'object' || Array.isArray(item)) {
    return false;
  }
  if (typeof item.type !== 'string' || item.type.trim() === '') {
    return false;
  }

  if (item.type === 'link') {
    if (typeof item.href !== 'string') {
      return false;
    }
    if (!Array.isArray(item.content)) {
      return false;
    }
    // Recursively validate, ensuring no nested links
    for (const nested of item.content) {
      if (!isValidInlineContent(nested) || nested.type === 'link') {
        return false;
      }
    }
  } else if (item.type === 'text') {
    if (typeof item.text !== 'string') {
      return false;
    }
  }

  if (item.styles !== undefined && (typeof item.styles !== 'object' || Array.isArray(item.styles))) {
    return false;
  }

  return true;
}

/**
 * Checks if a block is a valid BlockNote block object.
 * Enhanced with stricter validation to prevent ProseMirror renderSpec errors.
 */
export function isValidBlockNoteBlock(block: any): boolean {
  if (!block || typeof block !== 'object' || Array.isArray(block)) {
    return false;
  }
  if (typeof block.id !== 'string' || typeof block.type !== 'string') {
    return false;
  }

  // Validate content based on block type
  if (block.type === 'table') {
    if (block.content !== undefined && (typeof block.content !== 'object' || Array.isArray(block.content))) {
      return false;
    }
  } else {
    if (block.content !== undefined && !Array.isArray(block.content)) {
      return false;
    }
    if (Array.isArray(block.content) && !block.content.every(isValidInlineContent)) {
      return false;
    }
  }

  // Validate props - ensure no arrays in props (common renderSpec error cause)
  if (block.props !== undefined) {
    if (typeof block.props !== 'object' || Array.isArray(block.props)) {
      return false;
    }
    // Check each prop value to ensure no arrays are nested
    for (const key in block.props) {
      if (Array.isArray(block.props[key])) {
        return false;
      }
    }
  }

  // Validate children recursively
  if (block.children !== undefined) {
    if (!Array.isArray(block.children) || !block.children.every(isValidBlockNoteBlock)) {
      return false;
    }
  }

  return true;
}

/**
 * Sanitizes a single inline content item, returning a valid item or null.
 */
export function sanitizeInlineContent(item: any): any | null {
  if (!item || typeof item !== 'object' || Array.isArray(item)) return null;
  if (typeof item.type !== 'string') return null;

  const sanitized: any = {
    type: item.type,
    styles: (item.styles && typeof item.styles === 'object' && !Array.isArray(item.styles)) ? item.styles : {},
  };

  if (item.type === 'text') {
    sanitized.text = typeof item.text === 'string' ? item.text : '';
  } else if (item.type === 'link') {
    if (typeof item.href !== 'string' || !Array.isArray(item.content)) {
        // If link is malformed, try to convert it to plain text of its content
        const textContent = Array.isArray(item.content) ? item.content.map((i: any) => i.text).join('') : '';
        if (textContent) {
            return { type: 'text', text: textContent, styles: sanitized.styles };
        }
        return null;
    }
    sanitized.href = item.href;
    sanitized.content = item.content
      .map((subItem: any) => sanitizeInlineContent(subItem))
      .filter((subItem: any) => subItem && subItem.type !== 'link'); // No nested links
  } else if (item.type !== 'hardBreak') {
    // Unknown inline content type
    return null;
  }

  return sanitized;
}

/**
 * Sanitizes a single block, returning a valid block or null.
 * Enhanced to handle arrays in props and other ProseMirror incompatibilities.
 */
export function sanitizeBlock(block: any): any | null {
  if (!block || typeof block !== 'object' || Array.isArray(block)) return null;
  if (typeof block.id !== 'string' || typeof block.type !== 'string') return null;

  const sanitized: any = {
    id: block.id,
    type: block.type,
    props: {},
    content: [],
    children: [],
  };

  // Sanitize props - remove any arrays to prevent renderSpec errors
  if (block.props && typeof block.props === 'object' && !Array.isArray(block.props)) {
    for (const key in block.props) {
      const value = block.props[key];
      // Skip arrays entirely (common renderSpec error cause)
      if (!Array.isArray(value)) {
        sanitized.props[key] = value;
      }
    }
  }

  // Sanitize content
  if (block.type === 'table') {
    if (block.content && typeof block.content === 'object' && !Array.isArray(block.content)) {
      // Preserve table content but validate it's an object
      sanitized.content = block.content;
    } else {
      // Invalid table content - create valid empty table structure
      sanitized.content = { type: 'tableContent', columnWidths: [], rows: [] };
    }
  } else if (Array.isArray(block.content)) {
    sanitized.content = block.content.map(sanitizeInlineContent).filter(Boolean);
  }

  // Sanitize children recursively
  if (Array.isArray(block.children)) {
    sanitized.children = block.children.map(sanitizeBlock).filter(Boolean);
  }

  // Final validation check on the sanitized block
  if (!isValidBlockNoteBlock(sanitized)) {
    console.warn("Failed to sanitize block, dropping:", block, "Resulted in:", sanitized);
    return null;
  }

  return sanitized;
}

/**
 * Sanitizes an array of blocks.
 */
export function sanitizeInitialContent(content: any[] | undefined): any[] | undefined {
  if (!Array.isArray(content)) {
    return undefined;
  }
  const sanitized = content.map(sanitizeBlock).filter(Boolean);
  return sanitized.length > 0 ? sanitized : undefined;
}

/**
 * Validates an array of BlockNote blocks.
 * Checks ALL blocks to ensure none will cause renderSpec errors.
 */
export function isValidBlockNoteArray(arr: any[]): boolean {
  if (!arr || arr.length === 0) return false;
  // Check ALL blocks to ensure none will cause renderSpec errors
  for (let i = 0; i < arr.length; i++) {
    if (!isValidBlockNoteBlock(arr[i])) return false;
  }
  return true;
}

/**
 * Sanitizes an array of BlockNote blocks.
 * Maps over the array and sanitizes each block individually.
 */
export function sanitizeBlockNoteArray(arr: any[]): any[] {
  if (!Array.isArray(arr)) return [];
  const sanitized = arr.map(sanitizeBlock).filter(Boolean);
  return sanitized;
}

type SummaryFormat = 'legacy' | 'markdown' | 'blocknote';

/**
 * Detects the format of summary data.
 * Returns the format type and the normalized data object.
 */
export function detectSummaryFormat(data: any): { format: SummaryFormat; data: any } {
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
