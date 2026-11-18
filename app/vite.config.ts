import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri serves the dev build from a fixed port and expects a strict failure if
// it is taken -- a silent port shift would leave the webview pointing nowhere.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "safari15",
    sourcemap: false,
    chunkSizeWarningLimit: 900,
  },
});
