import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';

// package.json holds the app version (Tauri reads it too); shown as "v0.1" in the overlay.
const packageJson = fileURLToPath(new URL('./package.json', import.meta.url));
const { version } = JSON.parse(readFileSync(packageJson, 'utf8'));

/** Under `tauri dev` the version is read once at startup: restart the dev server (and
 *  so reload the pages with the new version) whenever package.json changes. */
function reloadOnVersionChange(): Plugin {
  return {
    name: 'reload-on-version-change',
    configureServer(server) {
      server.watcher.add(packageJson);
      server.watcher.on('change', (file) => {
        if (file === packageJson) void server.restart();
      });
    },
  };
}

export default defineConfig({
  // Native compiler output must not reload the web UI during `tauri dev`.
  server: { watch: { ignored: ['**/src-tauri/**'] } },
  define: { __APP_VERSION__: JSON.stringify(version) },
  plugins: [reloadOnVersionChange()],
  // Two pages: the overlay panel and the compass around the player.
  build: { rollupOptions: { input: { main: 'index.html', compass: 'compass.html' } } },
});
