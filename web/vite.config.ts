import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  worker: {
    format: "es",
  },
  server: {
    proxy: {
      "/api": {
        target: "http://127.0.0.1:9991",
        ws: true,
      },
    },
  },
  optimizeDeps: {
    include: ["vscode/localExtensionHost"],
  },
});
