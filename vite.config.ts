import { readFileSync } from 'node:fs';
import { defineConfig } from 'vite';

// package.json holds the app version (Tauri reads it too); shown as "v0.1" in the overlay.
const { version } = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'));

export default defineConfig({
  // Native compiler output must not reload the web UI during `tauri dev`.
  server: { watch: { ignored: ['**/src-tauri/**'] } },
  define: { __APP_VERSION__: JSON.stringify(version) },
  // Two pages: the overlay panel and the compass around the player.
  build: { rollupOptions: { input: { main: 'index.html', compass: 'compass.html' } } },
});
