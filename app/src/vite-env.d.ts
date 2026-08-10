/// <reference types="vite/client" />
/// <reference types="vite-plugin-wasm" />

declare module "*.worker.ts" {
	class WebpackWorker extends Worker {
		constructor();
	}
	export default WebpackWorker;
}
