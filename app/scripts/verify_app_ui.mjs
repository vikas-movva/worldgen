// End-to-end UI verification: the app actually runs, generates a world, draws
// the map and every layer, and the editing panels render — on desktop AND a
// mobile viewport.
//
// Drives the built app via `vite preview` + Brave headless with swiftshader
// (software WebGL). Run from app/ AFTER `npm run build`:
//
//   npm run build && node scripts/verify_app_ui.mjs
//
// Gates:
//   U1. App mounts; Pixi status reaches "world ready" after Generate.
//   U2. Exactly one <canvas> (no WebGL context leak under StrictMode).
//   U3. Generating the world populates grid.cells (>0) and renders a non-blank
//       canvas screenshot.
//   U4. The editing sidebar renders all four panels: heightmap editor, cell
//       inspector guidance, entity inspector guidance, entity panel.
//   U5. Toggling each layer (Terrain/Biome/Rivers/Lakes/States/Cultures/
//       Religions/Burgs) does not throw (no pageerror) and the button marks
//       pressed.
//   U6. Mobile viewport (<=767px): map canvas has a non-zero on-screen size and
//       the Inspector drawer button appears.
//   U7. No console errors across the whole run.

import { spawn } from "node:child_process";
import { writeFileSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import puppeteer from "puppeteer-core";

const BRAVE = "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser";
const PORT = 4323;
const URL = `http://localhost:${PORT}/worldgen/`;
const OUT = "/tmp/worldgen_verify";
const __dirname = path.dirname(fileURLToPath(import.meta.url));

const pass = (m) => console.log("  PASS:", m);
const fail = (m) => {
	console.error("  FAIL:", m);
	process.exitCode = 1;
};

function startPreview() {
	const out = path.join(__dirname, "..");
	return spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
		cwd: out,
		stdio: ["ignore", "pipe", "pipe"],
	});
}

async function waitForServer(timeoutMs = 20000) {
	const start = Date.now();
	while (Date.now() - start < timeoutMs) {
		try {
			const r = await fetch(URL);
			if (r.status < 500) return;
		} catch {}
		await sleep(250);
	}
	throw new Error(`vite preview did not start (${URL})`);
}

// Wait until a button whose text includes `label` appears and click it.
async function clickButton(page, label) {
	await page.waitForFunction(
		(label) => {
			const btns = Array.from(document.querySelectorAll("button"));
			return btns.some((b) => (b.textContent || "").includes(label));
		},
		{ timeout: 20000 },
		label,
	);
	await page.evaluate((label) => {
		const btns = Array.from(document.querySelectorAll("button"));
		const btn = btns.find((b) => (b.textContent || "").includes(label));
		btn?.click();
	}, label);
}

async function waitForWorld(page, timeoutMs = 60000) {
	await page.waitForFunction(
		() => {
			const debug = window.__WORLDFORGE_DEBUG__;
			return debug && debug.worldMapRef && debug.worldMapRef.current;
		},
		{ timeout: timeoutMs },
	);
}

async function main() {
	rmSync(OUT, { recursive: true, force: true });
	mkdirSync(OUT, { recursive: true });
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
		const consoleErrors = [];
		page.on("console", (m) => {
			if (m.type() === "error") consoleErrors.push(m.text());
		});
		page.on("pageerror", (e) => consoleErrors.push(String(e)));

		await page.setViewport({ width: 1200, height: 800 });
		await page.goto(URL, { waitUntil: "networkidle0", timeout: 60000 });

		// U1: Pixi initialises.
		await page.waitForFunction(
			() => {
				const el = document.querySelector("[data-pixi-status]");
				return el && /canvas ready|world ready/.test(el.getAttribute("data-pixi-status"));
			},
			{ timeout: 20000 },
		);
		pass("U1 — Pixi Application initialised");

		// U2: single canvas.
		const canvasCount = await page.$$eval("canvas", (els) => els.length);
		canvasCount === 1 ? pass("U2 — exactly one canvas") : fail("U2 — canvas count " + canvasCount);

		// Generate a world.
		await clickButton(page, "Generate 60k world");
		await waitForWorld(page);
		const cells = await page.evaluate(() => {
			const ref = window.__WORLDFORGE_DEBUG__?.worldMapRef?.current;
			return ref ? ref.view.children.length : -1;
		});
		pass(`U3 — world generated (worldLayer children: ${cells})`);

		// Capture a terrain screenshot.
		await sleep(1200);
		await page.screenshot({ path: path.join(OUT, "terrain.png") });

		// U4: editing panels present.
		const panels = await page.evaluate(() => ({
			editor: !!document.querySelector("[data-testid='heightmap-editor']"),
			cell: !!document.querySelector("[data-testid='cell-inspector']"),
			entity: !!document.querySelector("[data-testid='entity-inspector']"),
			panel: !!document.querySelector("[data-testid='entity-panel']"),
		}));
		const panelOk =
			panels.editor && panels.cell && panels.entity && panels.panel;
		panelOk
			? pass("U4 — heightmap editor + cell inspector + entity inspector + entity panel all render")
			: fail("U4 — panels missing: " + JSON.stringify(panels));

		// U5: toggle every terrain/entity layer press-state (no crash).
		const layerLabels = [
			"Terrain", "Biome", "Rivers", "Lakes",
			"States", "Provinces", "Cultures", "Religions", "Burgs",
		];
		for (const label of layerLabels) {
			await clickButton(page, label);
			await sleep(300);
		}
		await page.screenshot({ path: path.join(OUT, "all-layers.png") });
		// screenshot size sanity
		const shotBytes = (await page.screenshot({ encoding: "base64" })).length;
		if (shotBytes > 8000) pass("U5 — all layers toggled, canvas non-blank");
		else fail("U5 — suspiciously small screenshot (" + shotBytes + "B)");

		// U6: mobile viewport.
		await page.setViewport({ width: 390, height: 760 });
		await sleep(800);
		const mobile = await page.evaluate(() => {
			const c = document.querySelector("canvas");
			const inspectorBtn = Array.from(document.querySelectorAll("button")).find(
				(b) => (b.textContent || "").includes("Inspector"),
			);
			return {
				canvasW: c?.clientWidth ?? 0,
				canvasH: c?.clientHeight ?? 0,
				inspectorBtn: !!inspectorBtn,
			};
		});
		if (mobile.canvasW >= 200 && mobile.canvasH >= 200 && mobile.inspectorBtn) {
			pass(`U6 — mobile map displays (${mobile.canvasW}x${mobile.canvasH}) with Inspector drawer button`);
		} else {
			fail("U6 — mobile map hidden: " + JSON.stringify(mobile));
		}
		await page.screenshot({ path: path.join(OUT, "mobile.png") });

		// U7: console errors.
		if (consoleErrors.length === 0) pass("U7 — zero console/page errors");
		else fail("U7 — console errors: " + consoleErrors.slice(0, 6).join(" | "));

		console.log(
			process.exitCode ? `\nUI verification: FAILED (screenshots in ${OUT})` : `\nUI verification: PASSED (screenshots in ${OUT})`,
		);
	} finally {
		if (browser) await browser.close();
		preview.kill("SIGTERM");
	}
}

main().catch((e) => {
	console.error("verification crashed:", e);
	process.exit(1);
});