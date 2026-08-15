// Step 2.5.5 selection fix — visual verification via Brave headless + WebGL.
//
// Loads a minimal harness page (public/selection-harness.html) that imports
// WorldMap, builds a single-cell quad grid, calls setSelected, and renders one
// frame in a real WebGL context. Screenshots the canvas and measures the
// yellow (0xffff00) outline: it must be a thin hairline, NOT a giant filled
// polygon. Saves /tmp/step2.5.5_selection.png and prints the yellow pixel%
// and the WorldMap.getSelectionStrokeWidth() on-screen px value.
//
// Run from app/: node scripts/verify_selection.mjs (vite preview on 4321).

import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import puppeteer from "puppeteer-core";

const BRAVE = "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser";
const PORT = 4321;
const SHARED_BASE = "/worldgen/"; // vite config `base`
const URL = `http://localhost:${PORT}${SHARED_BASE}selection-harness.html`;

function startPreview() {
	// Use the vite DEV server (not preview) so root-level harness html files
	// are served + processed on the fly. (vite build/preview only emits the
	// configured index.html unless rollupOptions.input lists extras.)
	return spawn(
		"npx",
		["vite", "--port", String(PORT), "--strictPort", "--host", "127.0.0.1"],
		{ cwd: process.cwd(), stdio: ["ignore", "pipe", "pipe"] },
	);
}

async function waitForServer(timeoutMs = 20000) {
	const start = Date.now();
	while (Date.now() - start < timeoutMs) {
		try {
			if ((await fetch(URL)).ok) return;
		} catch {}
		await sleep(250);
	}
	throw new Error("vite preview did not start in time");
}

const pass = (m) => console.log("  PASS:", m);
const fail = (m) => {
	console.error("  FAIL:", m);
	process.exitCode = 1;
};

async function main() {
	const preview = startPreview();
	let browser;
	try {
		await waitForServer();
		browser = await puppeteer.launch({
			executablePath: BRAVE,
			headless: "new",
			args: [
				"--no-sandbox",
				"--use-gl=angle",
				"--use-angle=swiftshader",
				"--enable-unsafe-swiftshader",
				"--ignore-gpu-blocklist",
				"--enable-webgl",
				"--disable-gpu-sandbox",
			],
		});
		const page = await browser.newPage();
		const errors = [];
		// Resource 404s (favicon.ico, etc.) arrive as console errors without
		// a stack; real JS errors arrive as pageerror with a stack. Ignore
		// bare 404 resource messages but keep pageerror.
		const isResource404 = (t) => /404|favicon|Failed to load resource/.test(t);
		page.on("console", (m) => {
			if (m.type() === "error" && !isResource404(m.text())) {
				errors.push(m.text());
			}
		});
		page.on("pageerror", (e) => errors.push(String(e)));

		await page.setViewport({ width: 600, height: 400 });
		await page.goto(URL, { waitUntil: "networkidle0", timeout: 30000 });
		// Wait for the harness to set window.__selResult.
		const res = await page.waitForFunction(
			() => window.__selResult,
			{ timeout: 15000 },
		).then((h) => h.jsonValue());

		// Screenshot the rendered canvas for human inspection.
		const shot = await page.screenshot({ encoding: "base64", type: "png" });
		const bytes = Buffer.from(shot, "base64");
		const fs = await import("node:fs");
		fs.writeFileSync("/tmp/step2.5.5_selection.png", bytes);

		// The harness counted yellow (0xffff00) pixels via gl.readPixels
		// (WebGL framebuffer readback; a canvas can't expose both a 2d and
		// a webgl context, so the count must come from the harness side).
		const yellowPct = res.yellowPct;

		const strokePx = res.strokeWidthPx;
		const ok = strokePx > 1.5 && strokePx < 2.5;
		if (ok) {
			pass(
				`on-screen stroke width ${strokePx.toFixed(2)}px (thin hairline, not ~2560px polygon)`,
			);
		} else {
			fail(
				`stroke width ${strokePx} px is outside the ~2px hairline range`,
			);
		}
		// A ~2px outline around a moderate quad is well under 1% of a
		// 600x400 canvas. A filled cell would be several% or more. Also
		// assert the harness actually read pixels (yellowPct >= 0).
		if (yellowPct < 0) {
			fail("harness gl.readPixels returned no data (yellowPct < 0)");
		} else if (yellowPct < 1.0) {
			pass(
				`yellow pixels ${yellowPct.toFixed(3)}% of canvas (outline, not a filled polygon)`,
			);
		} else {
			fail(
				`yellow pixel ratio ${yellowPct.toFixed(2)}% — too high, selection looks filled`,
			);
		}
		if (errors.length === 0) pass("zero console errors");
		else fail(`console errors: ${errors.slice(0, 5).join(" | ")}`);
	} finally {
		if (browser) await browser.close();
		preview.kill("SIGTERM");
	}
}

main().catch((e) => {
	console.error("verification crashed:", e);
	process.exit(1);
});
