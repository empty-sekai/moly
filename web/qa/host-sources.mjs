import { parseArgs } from "node:util";
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
const { values } = parseArgs({
  options: { origin: { type: "string" }, out: { type: "string" } },
});
if (!values.origin || !values.out) throw Error("Supply --origin and --out");
const origin = new URL(values.origin).origin,
  out = path.resolve(values.out);
await fs.mkdir(out, { recursive: true });
const manifest = await fetch(origin + "/moly/manifest.json").then((r) =>
  r.json(),
);
const jp = manifest.snapshots.find((s) => s.region === "jp" && s.available);
assert.ok(jp);
const index = await fetch(origin + jp.catalog).then((r) => r.json());
const { chromium } = await import(
  process.env.MOLY_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch({
  headless: true,
  channel: process.env.MOLY_BROWSER_CHANNEL || "chrome",
});
const page = await browser.newPage({ viewport: { width: 1280, height: 960 } });
page.setDefaultTimeout(25000);
const errors = [],
  events = [];
let failed = false;
const record = (type, data) => {
  events.push({ type, data });
  console.log(type, JSON.stringify(data).slice(0, 1800));
};
page.on("pageerror", (e) => errors.push(String(e)));
page.on("response", (r) => {
  if (r.status() >= 400 && new URL(r.url()).origin === origin)
    errors.push(`${r.status()} ${r.url()}`);
});
await page.addInitScript(() => {
  window.__sourceQA = { last: null, closed: [] };
  addEventListener("message", (e) => {
    if (
      e.origin !== location.origin ||
      e.source !==
        document.querySelector(".interaction-runtime iframe")?.contentWindow ||
      e.data?.source !== "moly"
    )
      return;
    if (e.data.type === "snapshot") window.__sourceQA.last = e.data.value;
    if (e.data.type === "closed") window.__sourceQA.closed.push(e.data.value);
  });
});
async function enter() {
  await page
    .frameLocator(".interaction-runtime iframe")
    .locator('#stage-start[data-moly-ready="true"]')
    .waitFor({ timeout: 60000 });
  await page.locator('[data-action="play-selected"]').click();
}
try {
  await page.goto(
    origin +
      "/en-us/mysekai/interactions/?region=cn&fixture=15&content=fixture:15&tab=furniture",
    { waitUntil: "domcontentloaded", timeout: 60000 },
  );
  await enter();
  await page.waitForFunction(
    () => window.__sourceQA.last?.status.phase === "playing",
    null,
    { timeout: 60000 },
  );
  await page
    .locator(".interaction-source-options button")
    .filter({ hasText: /^JP$/ })
    .click();
  await page.waitForFunction(
    () =>
      new URL(location.href).searchParams.get("region") === "jp" &&
      document
        .querySelector(".interaction-runtime iframe")
        ?.src.includes("region=jp"),
  );
  await page.locator(".interaction-card").first().waitFor();
  let url = new URL(page.url());
  assert.equal(url.searchParams.get("snapshot"), jp.id);
  assert.equal(url.searchParams.get("fixture"), null);
  assert.equal(url.searchParams.get("content"), null);
  assert.ok(
    (await page.evaluate(() => window.__sourceQA.closed)).some(
      (v) => v.restored === true,
    ),
  );
  const cards = await page
    .locator(".interaction-card")
    .evaluateAll((nodes) =>
      nodes.map((n) => n.getAttribute("data-content-key")),
    );
  const key = cards.find((key) =>
    index.entries.some((e) => e.key === key && e.available),
  );
  assert.ok(key, "No real playable JP entry in first page");
  await page.locator(`[data-content-key="${key}"]`).click();
  await enter();
  await page.waitForFunction(
    (key) =>
      window.__sourceQA.last?.region === "jp" &&
      window.__sourceQA.last.status.activeKey === key &&
      window.__sourceQA.last.status.phase === "playing",
    key,
    { timeout: 60000 },
  );
  const src = new URL(
    await page.locator(".interaction-runtime iframe").getAttribute("src"),
  );
  assert.equal(src.searchParams.get("assets"), jp.assets);
  assert.equal(src.searchParams.get("snapshot"), jp.id);
  record("explicit-jp-switch", {
    key,
    iframe: src.href,
    state: await page.evaluate(() => ({
      region: window.__sourceQA.last.region,
      version: window.__sourceQA.last.version,
      status: window.__sourceQA.last.status,
    })),
  });
  await page.screenshot({ path: path.join(out, "host-explicit-jp.png") });
  await page.getByRole("button", { name: "Close player", exact: true }).click();
  await page.waitForFunction(
    () => !document.querySelector(".interaction-runtime iframe"),
  );
  for (const region of ["en", "tw", "kr"]) {
    await page.goto(
      origin +
        `/en-us/mysekai/interactions/?region=${region}&fixture=157&content=fixture:157&tab=furniture`,
      { waitUntil: "domcontentloaded" },
    );
    await page.locator(".interaction-notice").waitFor();
    assert.equal(
      await page.locator(".interaction-source-bar strong").innerText(),
      region.toUpperCase(),
    );
    assert.equal(await page.locator(".interaction-runtime iframe").count(), 0);
    assert.equal(await page.locator(".interaction-card").count(), 0);
    assert.equal(new URL(page.url()).searchParams.get("region"), region);
    record("unsupported-remains-explicit", region);
  }
  await page
    .locator(".interaction-source-options button")
    .filter({ hasText: /^CN$/ })
    .click();
  await page.locator(".interaction-card").first().waitFor();
  url = new URL(page.url());
  assert.equal(url.searchParams.get("fixture"), null);
  assert.equal(url.searchParams.get("content"), null);
  // A fresh narrow context avoids inheriting a desktop-open sidebar state.
  const a11y = await browser.newContext({
    viewport: { width: 760, height: 1024 },
    colorScheme: "dark",
    reducedMotion: "reduce",
    contrast: "more",
  });
  const view = await a11y.newPage();
  view.on("pageerror", (error) => errors.push(String(error)));
  const client = await a11y.newCDPSession(view);
  await client.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "prefers-color-scheme", value: "dark" },
      { name: "prefers-reduced-motion", value: "reduce" },
      { name: "prefers-reduced-transparency", value: "reduce" },
      { name: "prefers-contrast", value: "more" },
    ],
  });
  await view.goto(origin + "/en-us/mysekai/interactions/?region=cn", {
    waitUntil: "domcontentloaded",
  });
  await view.locator(".interaction-card").first().waitFor();
  const preferences = await view.evaluate(() => ({
    motion: matchMedia("(prefers-reduced-motion: reduce)").matches,
    transparency: matchMedia("(prefers-reduced-transparency: reduce)").matches,
    contrast: matchMedia("(prefers-contrast: more)").matches,
    overflow:
      document.documentElement.scrollWidth -
      document.documentElement.clientWidth,
  }));
  assert.ok(
    preferences.motion && preferences.transparency && preferences.contrast,
  );
  assert.equal(preferences.overflow, 0);
  await view.screenshot({
    path: path.join(out, "host-760-reduced-contrast.png"),
  });
  record("display-preferences", preferences);
  await a11y.close();
  assert.deepEqual(errors, []);
} catch (error) {
  failed = true;
  record("failure", String(error));
  await page
    .screenshot({ path: path.join(out, "host-sources-failure.png") })
    .catch(() => {});
} finally {
  await fs.writeFile(
    path.join(out, "host-sources.json"),
    JSON.stringify({ failed, errors, events }, null, 2),
  );
  await browser.close();
  if (failed) process.exitCode = 1;
}
