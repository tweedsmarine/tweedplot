import { defineConfig } from "vite";

// Tauri expects a fixed port and no screen clearing.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", outDir: "dist", chunkSizeWarningLimit: 2000, rollupOptions: { input: { main: "index.html", cockpit: "cockpit.html" } } },
  worker: { format: "es" },
});
