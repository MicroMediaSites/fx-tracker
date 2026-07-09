import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import path from 'path';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify('0.0.0-e2e'),
    __BUILD_MODE__: JSON.stringify('development'),
  },
  clearScreen: false,
  server: {
    port: 1422,
    strictPort: true,
  },
  resolve: {
    alias: {
      '@tauri-apps/plugin-updater': path.resolve(__dirname, 'e2e/mocks/tauri-plugins.ts'),
      '@tauri-apps/plugin-process': path.resolve(__dirname, 'e2e/mocks/tauri-plugins.ts'),
      '@tauri-apps/plugin-shell': path.resolve(__dirname, 'e2e/mocks/tauri-plugins.ts'),
      '@tauri-apps/plugin-notification': path.resolve(__dirname, 'e2e/mocks/tauri-plugins.ts'),
      '@tauri-apps/api/core': path.resolve(__dirname, 'e2e/mocks/tauri-api-core.ts'),
      '@tauri-apps/api/event': path.resolve(__dirname, 'e2e/mocks/tauri-api-event.ts'),
      '@tauri-apps/api/window': path.resolve(__dirname, 'e2e/mocks/tauri-api-window.ts'),
      '@tauri-apps/api/webviewWindow': path.resolve(__dirname, 'e2e/mocks/tauri-api-webview-window.ts'),
    },
  },
  build: {
    outDir: 'build',
    emptyOutDir: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
});
