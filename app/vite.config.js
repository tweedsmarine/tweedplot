import { defineConfig } from "vite";
import { execSync } from "node:child_process";

// the commit the app was built from, for Copy for support
let build = "dev";
try { build = execSync("git rev-parse --short HEAD", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim(); } catch { /* not a git checkout */ }

// Tauri expects a fixed port and no screen clearing.
export default defineConfig({
  clearScreen: false,
  define: { __BUILD__: JSON.stringify(build) },
  server: { port: 1420, strictPort: true },
  build: { target: "es2022", outDir: "dist", chunkSizeWarningLimit: 2000, rollupOptions: { input: { main: "index.html" } } },
  worker: { format: "es" },
});
