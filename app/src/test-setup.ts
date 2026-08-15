// Vitest setup — runs before each spec file in the jsdom environment.
//
// React 19's `act(...)` requires the `IS_REACT_ACT_ENVIRONMENT` global to be
// truthy so it knows it's running inside a test environment that flushes
// effects synchronously. Otherwise React prints the "The current testing
// environment is not configured to support act(...)" warning, even though
// the updates still flush. These are component-level tests (CellInspector);
// the existing store/bridge tests are plain TS and are unaffected.
//
// See https://react.dev/reference/react/act#usage

(
	globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;
