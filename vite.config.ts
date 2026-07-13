import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and no auto-clearing of its own logs.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  // Only expose TAURI_* env vars to the frontend bundle.
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "chrome110",
    sourcemap: false,
  },
});
