import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import wasmFn from "vite-plugin-wasm";

// https://vite.dev/config/
export default defineConfig({
	// Served from https://vikas-movva.github.io/worldgen/ — must match the repo slug
	// so built assets resolve on the subpath (GitHub Pages has no SPA rewrite).
	base: "/worldgen/",
	plugins: [
		react(),
		// @ts-expect-error - vite-plugin-wasm default export is callable at runtime
		wasmFn(),
	],
	worker: {
		format: "es",
		// @ts-expect-error - vite-plugin-wasm default export is callable at runtime
		plugins: () => [wasmFn()],
	},
});
