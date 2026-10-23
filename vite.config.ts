import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed port and no screen clearing.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  esbuild: { target: 'es2022' },
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 2000,
  },
});
