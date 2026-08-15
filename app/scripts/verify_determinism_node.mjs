// Determinism gate (tech-reqs §4) — node-wasm leg.
// Builds the world twice via the node-target WASM, serializes the Grid to JSON,
// and xxHash64s it (pure-JS xxHash64, seed 0 — same algorithm the Rust
// `cargo test` leg uses, so node and native agree on the digest). Asserts:
//   - same seed → identical digest (byte-identical world)
//   - different seed → different digest
//   - digest is non-trivial (not 0)
// This mirrors what the cross-browser (Playwright chromium + firefox) leg will
// assert once that job is wired; see `.github/workflows/ci.yml`.
import { createRequire } from "node:module";
import { performance } from "node:perf_hooks";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

// ---- xxHash64 (public-domain algorithm, seed 0) ----
const PRIME64_1 = 0x9e3779b185ebca87n;
const PRIME64_2 = 0xc2b2ae3d27d4eb4fn;
const PRIME64_3 = 0x165667b19e3779f9n;
const PRIME64_4 = 0x85ebca77c2b2ae63n;
const PRIME64_5 = 0x27d4eb2f165667c5n;
const MASK64 = 0xffffffffffffffffn;

function rotl(x, r) {
	return ((x << BigInt(r)) | (x >> (BigInt(64) - BigInt(r)))) & MASK64;
}
function u64(a, b) {
	// combine two u32 into u64
	return (BigInt(a >>> 0) | (BigInt(b >>> 0) << 32n)) & MASK64;
}
function xxh64(buf, seed) {
	const n = buf.length;
	let h;
	let v1, v2, v3, v4;
	let i = 0;
	const seedB = BigInt(seed >>> 0);
	if (n >= 32) {
		v1 = (seedB + PRIME64_1 + PRIME64_2) & MASK64;
		v2 = (seedB + PRIME64_2) & MASK64;
		v3 = seedB & MASK64;
		v4 = (seedB - PRIME64_1) & MASK64;
		const limit = n - 31;
		for (; i < limit; i += 32) {
			const r = (j) => u64(buf.readUInt32LE(j), buf.readUInt32LE(j + 4));
			v1 = (rotl((v1 + r(i) * PRIME64_2) & MASK64, 31n) * PRIME64_1) & MASK64;
			v2 =
				(rotl((v2 + r(i + 8) * PRIME64_2) & MASK64, 31n) * PRIME64_1) & MASK64;
			v3 =
				(rotl((v3 + r(i + 16) * PRIME64_2) & MASK64, 31n) * PRIME64_1) & MASK64;
			v4 =
				(rotl((v4 + r(i + 24) * PRIME64_2) & MASK64, 31n) * PRIME64_1) & MASK64;
		}
		h =
			(rotl(v1, 1n) + rotl(v2, 7n) + rotl(v3, 12n) + rotl(v4, 18n)) &
			MASK64 &
			MASK64;
		h = (h + v1) & MASK64;
		h = (h + v2) & MASK64;
		h = (h + v3) & MASK64;
		h = (h + v4) & MASK64;
	} else {
		h = (seedB + PRIME64_5) & MASK64;
	}
	h = (h + BigInt(n)) & MASK64;
	const limit = n - 7;
	for (; i < limit; i += 8) {
		const k = u64(buf.readUInt32LE(i), buf.readUInt32LE(i + 4));
		h = (h ^ (rotl((k * PRIME64_3) & MASK64, 37n) * PRIME64_4)) & MASK64;
		h = (rotl(h, 27n) * PRIME64_1 + PRIME64_4) & MASK64;
	}
	if (i + 3 < n) {
		h = (h ^ (u64(buf.readUInt32LE(i), 0) * PRIME64_3)) & MASK64;
		h = (rotl(h, 33n) * PRIME64_4 + PRIME64_2) & MASK64;
		i += 4;
	}
	for (; i < n; i++) {
		h = (h ^ (BigInt(buf[i]) * PRIME64_5)) & MASK64;
		h = rotl(h, 11n) * PRIME64_1;
	}
	h = (h ^ (h >> 33n)) & MASK64;
	h = (h * PRIME64_2) & MASK64;
	h = (h ^ (h >> 29n)) & MASK64;
	h = (h * PRIME64_3) & MASK64;
	h = (h ^ (h >> 32n)) & MASK64;
	return h.toString(16).padStart(16, "0");
}

const N = process.env.N ? parseInt(process.env.N, 10) : 60000;
const SEED = 42;
const opts = {
	map_size: 100,
	latitude: 50,
	longitude: 50,
	prec: 100,
	height_exponent: 2.0,
	temperature_equator: 27,
	temperature_north_pole: -30,
	temperature_south_pole: -15,
	winds: [225, 45, 225, 315, 135, 315],
};

const t0 = performance.now();
const g1 = wasm.generate_world(SEED, N, opts);
const g2 = wasm.generate_world(SEED, N, opts);
const j1 = Buffer.from(JSON.stringify(g1));
const j2 = Buffer.from(JSON.stringify(g2));
const tTotal = performance.now() - t0;

const h1 = xxh64(j1, 0);
const h2 = xxh64(j2, 0);
const identical = h1 === h2;

const g3 = wasm.generate_world(SEED + 1, N, opts);
const h3 = xxh64(Buffer.from(JSON.stringify(g3)), 0);
const differs = h1 !== h3;

console.log(`determinism node: serialize+hash ${tTotal.toFixed(0)}ms`);
console.log(`xxh64(seed=${SEED}) run1 = ${h1}`);
console.log(`xxh64(seed=${SEED}) run2 = ${h2}`);
console.log(`xxh64(seed=${SEED + 1})    = ${h3}`);
console.log(`same-seed byte-identical: ${identical}`);
console.log(`different-seed differs: ${differs}`);
console.log(`non-trivial: ${h1 !== "0000000000000000"}`);

const PASS = identical && differs && h1 !== "0000000000000000";
console.log(`VERDICT: ${PASS ? "PASS" : "FAIL"}`);
process.exit(PASS ? 0 : 1);
