import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import wasmFn from "vite-plugin-wasm";

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    react(),
    // @ts-ignore - vite-plugin-wasm default export is callable at runtime
    wasmFn(),
  ],
  worker: {
    format: "es",
    // @ts-ignore - vite-plugin-wasm default export is callable at runtime
    plugins: () => [wasmFn()],
  },
});