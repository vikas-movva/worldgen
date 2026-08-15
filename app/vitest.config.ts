import { defineConfig } from "vitest/config";

// Unit tests for the core worker bridge (Step 2.1) and clamp helpers. These
// run in a jsdom environment so the optional `Worker` / `Uint8Array` globals
// exist, but the bridge is exercised with an injected fake worker — no real
// Web Worker or WASM module is required.
export default defineConfig({
	test: {
		environment: "jsdom",
		include: ["src/**/*.{test,spec}.{ts,tsx}"],
		// Setup file enables React `act(...)` environment for component specs.
		setupFiles: ["./src/test-setup.ts"],
		// Keep output compact; the bridge tests are fast and synchronous-ish.
		silent: false,
	},
});
