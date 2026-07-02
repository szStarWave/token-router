import { defineConfig } from 'vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import babel from '@rolldown/plugin-babel'

export default defineConfig(({ mode }) => ({
  plugins: [
    react(),
    babel({ presets: [reactCompilerPreset()] }),
  ],
  clearScreen: false,
  // Release builds always disable test-only browser features (DevTools, refresh, etc.)
  define:
    mode === 'production'
      ? { 'import.meta.env.VITE_FLOWY_TEST_SERVER': JSON.stringify('') }
      : undefined,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
}))
