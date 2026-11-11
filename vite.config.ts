import { defineConfig } from 'vite';

export default defineConfig({
  // Native compiler output must not reload the web UI during `tauri dev`.
  server: { watch: { ignored: ['**/src-tauri/**'] } },
});
