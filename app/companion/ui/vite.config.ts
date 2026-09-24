import { defineConfig } from "vite";

// The Tauri shell loads this frontend either from the dev server (devUrl
// below, started with `npm run dev`) or from the built `dist/` directory.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    outDir: "dist",
    sourcemap: false,
  },
});
