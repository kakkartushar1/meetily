import { defineConfig } from 'vitest/config';
import path from 'path';

export default defineConfig({
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./src/__tests__/setup.ts'],
    include: ['src/__tests__/**/*.{test,spec}.{ts,tsx}'],
    css: false,
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  // Use oxc for TS and esbuild for TSX (JSX support)
  oxc: false,
  esbuild: {
    jsx: 'automatic',
    include: /\.[jt]sx?$/,
  },
});
