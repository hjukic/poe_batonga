import { defineConfig } from 'vite';

export default defineConfig({
  // Native compiler output must not reload the web UI during `tauri dev`.
  server: { watch: { ignored: ['**/src-tauri/**'] } },
  // Two pages: the overlay panel and the compass around the player.
  build: { rollupOptions: { input: { main: 'index.html', compass: 'compass.html' } } },
});
