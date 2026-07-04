import { defineConfig } from "vite";

// Fixed port so Tauri's devUrl is stable.
export default defineConfig({
  clearScreen: false,
  server: { port: 1430, strictPort: true },
});
