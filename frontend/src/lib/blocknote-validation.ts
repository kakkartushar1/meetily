
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

  // Validate props
  if (block.props !== undefined && (typeof block.props !== 'object' || Array.isArray(block.props))) {
    return false;
  }

  // Validate children
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
        const textContent = Array.isArray(item.content) ? item.content.map(i => i.text).join('') : '';
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
 */
export function sanitizeBlock(block: any): any | null {
  if (!block || typeof block !== 'object' || Array.isArray(block)) return null;
  if (typeof block.id !== 'string' || typeof block.type !== 'string') return null;

  const sanitized: any = {
    id: block.id,
    type: block.type,
    props: (block.props && typeof block.props === 'object' && !Array.isArray(block.props)) ? block.props : {},
    content: [],
    children: [],
  };

  // Sanitize content
  if (block.type === 'table') {
    if (block.content && typeof block.content === 'object' && !Array.isArray(block.content)) {
      // A simple copy is probably not enough, but for now, we preserve it as requested.
      // A proper sanitizer would validate the TableContent structure.
      sanitized.content = block.content;
    } else {
      // Invalid table content, this was the root cause.
      // Don't force to [], create a valid empty table structure.
      sanitized.content = { type: 'tableContent', rows: [] };
    }
  } else if (Array.isArray(block.content)) {
    sanitized.content = block.content.map(sanitizeInlineContent).filter(Boolean);
  }
  // If content is something else, it will remain an empty array.

  // Sanitize children
  if (Array.isArray(block.children)) {
    sanitized.children = block.children.map(sanitizeBlock).filter(Boolean);
  }

  // Final validation check on the sanitized block
  if (!isValidBlockNoteBlock(sanitized)) {
    // This shouldn't happen if sanitization is correct, but as a safeguard:
    console.error("Failed to sanitize block, dropping:", block, "Resulted in:", sanitized);
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
