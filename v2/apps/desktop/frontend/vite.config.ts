import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import UnoCSS from 'unocss/vite'

export default defineConfig(({ mode }) => ({
  plugins: [
    react({ compiler: true }),
    // OpenChamber 规范预设 + 应用组合件（见 uno.config.ts）
    UnoCSS(),
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
