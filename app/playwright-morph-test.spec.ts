import { test, expect } from '@playwright/test';

test('seekbar drag morphs the map', async ({ page }) => {
  const consoleLogs: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'log' || msg.type() === 'error' || msg.type() === 'warning') {
      consoleLogs.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on('pageerror', (err) => {
    consoleLogs.push(`[PAGEERROR] ${err.message}`);
  });

  // Navigate to the dev server
  await page.goto('http://localhost:4399');
  await page.waitForTimeout(3000);

  // Wait for the canvas to appear
  await page.waitForSelector('canvas', { timeout: 30000 });
  consoleLogs.push(`[INFO] Canvas found`);

  // Wait for timeline to appear
  await page.waitForSelector('[data-testid="timeline-scrubber"]', { timeout: 30000 });
  consoleLogs.push(`[INFO] Timeline scrubber found`);

  // Wait for the timeline to generate and project
  await page.waitForTimeout(5000);

  // Find the seekbar
  const seekbar = await page.locator('input[type="range"]');
  await expect(seekbar).toBeVisible();

  // Read the seekbar attributes
  const min = await seekbar.getAttribute('min');
  const max = await seekbar.getAttribute('max');
  consoleLogs.push(`[INFO] Seekbar min=${min} max=${max}`);

  // Drag the seekbar to a new position
  const box = await seekbar.boundingBox();
  if (box) {
    const targetX = box.x + box.width * 0.5; // drag to 50%
    const targetY = box.y + box.height / 2;
    await page.mouse.move(targetX, targetY);
    await page.mouse.down();
    await page.mouse.up();
    consoleLogs.push(`[INFO] Dragged seekbar to 50%`);
  }

  // Wait for the morph to happen
  await page.waitForTimeout(5000);

  // Output all console logs
  consoleLogs.push(`[INFO] Total console logs: ${consoleLogs.length}`);
  consoleLogs.forEach(log => console.log(log));
});
