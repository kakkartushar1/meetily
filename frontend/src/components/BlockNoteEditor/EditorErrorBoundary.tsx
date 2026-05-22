"use client";

import { Component, ReactNode } from "react";

/**
 * Error boundary component to catch ProseMirror renderSpec errors
 * that occur during BlockNote editor rendering.
 */
class EditorErrorBoundary extends Component<
  { children: ReactNode; onError: (error: Error) => void },
  { hasError: boolean; error: Error | null }
> {
  constructor(props: { children: ReactNode; onError: (error: Error) => void }) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error) {
    console.error('❌ EDITOR: BlockNote rendering error caught by boundary:', error.message);
    this.props.onError(error);
  }

  render() {
    if (this.state.hasError) {
      return null; // Parent component handles the error display
    }
    return this.props.children;
  }
}

export default EditorErrorBoundary;