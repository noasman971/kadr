// Vite builds the Svelte front end. Tauri loads it from the development server
// (`npm run tauri dev`) or from `dist/` once built (`npm run tauri build`).
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

// Tauri expects a fixed port, and fails if it is taken rather than moving on.
const DEVELOPMENT_SERVER_PORT = 1420;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: DEVELOPMENT_SERVER_PORT,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // WebView2 (Windows), WebKitGTK (Linux) and WKWebView (macOS) are all recent.
    target: "es2022",
    sourcemap: false,
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
