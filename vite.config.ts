import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'fs';

const host = process.env.TAURI_DEV_HOST;

// Read version from tauri.conf.json (source of truth for app version)
const tauriConfig = JSON.parse(readFileSync('./src-tauri/tauri.conf.json', 'utf-8'));

// Build mode: 'development' | 'staging' | 'production'
// Set via VITE_BUILD_MODE env var in CI, defaults to 'development' locally
const buildMode = process.env.VITE_BUILD_MODE || 'development';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify(tauriConfig.version),
    __BUILD_MODE__: JSON.stringify(buildMode),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**', '**/.claude/**', '**/node_modules/**', '**/.git/**', '**/docs/**']
    }
  },
  build: {
    outDir: 'build',
    emptyOutDir: true,
    // Strip console.log/warn/debug in production builds (keep console.error for error reporting)
    ...(buildMode === 'production' && {
      minify: 'esbuild',
    }),
  },
  esbuild: {
    // In production, treat these as pure (side-effect-free) so esbuild removes them
    ...(buildMode === 'production' && {
      pure: ['console.log', 'console.warn', 'console.debug', 'console.info'],
    }),
  },
  envPrefix: ['VITE_', 'TAURI_']
});
