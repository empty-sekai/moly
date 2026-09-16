// Real Chromium acceptance of the host's automatic, scoped resource cache.
// The site shell itself is intentionally outside the service worker scope.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
const { values: options } = parseArgs({
  options: {
    origin: { type: "string", default: "http://127.0.0.1:8082" },
    region: { type: "string", default: "cn" },
    locale: { type: "string" },
    out: { type: "string" },
  },
});
assert.ok(options.out, "--out must name an evidence directory outside source");
assert.ok(["cn", "jp"].includes(options.region));
const out = path.resolve(options.out);
await fs.mkdir(out, { recursive: true });
const { chromium } = await import(
  process.env.MOLY_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch({
  headless: true,
  channel: process.env.MOLY_BROWSER_CHANNEL || "chrome",
});
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
});
const page = await context.newPage();
const events = [],
  failures = [],
  httpFailures = [];
const record = (type, data) => {
  events.push({ type, data });
  console.log(type, JSON.stringify(data));
};
page.on("pageerror", (error) => failures.push(String(error)));
page.on("response", (response) => {
  if (response.status() >= 400)
    httpFailures.push({ url: response.url(), status: response.status() });
});
const region = options.region,
  locale = options.locale || (region === "cn" ? "zh-cn" : "ja-jp");
assert.ok(["zh-cn", "ja-jp", "en-us"].includes(locale));
const key = region === "cn" ? "talk:fixture:1374" : "talk:general:3912";
const entry = `${options.origin}/${locale}/mysekai/interactions/?region=${region}&content=${encodeURIComponent(key)}`;
let frame;
async function cacheCommand(type = "query", enabled) {
  return page.evaluate(
    async ({ type, enabled }) => {
      const registration =
        await navigator.serviceWorker.getRegistration("/moly/");
      if (!registration?.active) throw Error("Resource worker is not active");
      return new Promise((resolve, reject) => {
        const channel = new MessageChannel();
        const timer = setTimeout(() => {
          channel.port1.close();
          reject(Error("Cache command timed out"));
        }, 30000);
        channel.port1.onmessage = ({ data }) => {
          clearTimeout(timer);
          channel.port1.close();
          resolve(data);
        };
        registration.active.postMessage(
          { source: "moly-cache-host", schemaVersion: 1, type, enabled },
          [channel.port2],
        );
      });
    },
    { type, enabled },
  );
}
async function prepared() {
  const iframe = page.locator(".interaction-runtime iframe").first();
  await iframe.waitFor({ timeout: 60000 });
  frame = await (await iframe.elementHandle()).contentFrame();
  await frame.waitForFunction(
    () => document.querySelector("#stage-start")?.dataset.molyReady === "true",
    null,
    { timeout: 90000 },
  );
  await frame.evaluate(async () => {
    const url = performance
      .getEntriesByType("resource")
      .find((row) => /moly-app\.js$/.test(row.name))?.name;
    if (!url) throw Error("WASM JavaScript module was not loaded");
    window.__qa = await import(url);
  });
  assert.equal((await cacheCommand()).ok, true);
}
async function play() {
  await page.locator('[data-action="play-selected"]').click();
  await frame.waitForFunction(
    (key) => {
      const state = JSON.parse(window.__qa.library_snapshot());
      return (
        state.status.activeKey === key &&
        state.status.phase === "playing" &&
        JSON.parse(window.__qa.library_diagnostics()).transcript?.visible
      );
    },
    key,
    { timeout: 60000 },
  );
}
async function completeDialogue(expected) {
  const seen = [],
    start = Date.now();
  let lastClick = 0;
  while (Date.now() - start < 65000) {
    const current = await frame.evaluate(() => ({
      state: JSON.parse(window.__qa.library_snapshot()),
      diagnostics: JSON.parse(window.__qa.library_diagnostics()),
    }));
    if (current.state.status.phase === "error")
      throw Error(current.state.status.error);
    const line = current.diagnostics.transcript;
    if (line?.visible && line.text?.trim()) {
      if (
        !seen.some(
          (row) => row.text === line.text && row.speaker === line.speaker,
        )
      )
        seen.push({ speaker: line.speaker, text: line.text });
      if (!line.typing && Date.now() - lastClick > 1000) {
        const canvas = frame.locator("#app-canvas"),
          box = await canvas.boundingBox();
        assert.ok(box);
        await canvas.click({
          position: { x: box.width * 0.82, y: box.height * 0.83 },
        });
        lastClick = Date.now();
      }
    }
    if (
      current.state.status.phase === "idle" &&
      !current.state.status.canStop &&
      !current.diagnostics.independent
    ) {
      for (const line of expected)
        assert.ok(
          seen.some(
            (row) => row.text === line.text && row.speaker === line.speaker,
          ),
          `Source line missing: ${JSON.stringify(line)}`,
        );
      assert.ok(seen.length);
      return { naturalCompletion: true, lines: seen.length };
    }
    await page.waitForTimeout(125);
  }
  throw Error("Cached dialogue did not finish naturally");
}
async function verifyBaseOffline(descriptor) {
  return frame.evaluate(async (descriptor) => {
    const response = await fetch(descriptor, { cache: "no-store" });
    if (!response.ok || response.headers.get("X-Moly-Cache") !== "retained")
      throw Error("Descriptor is not retained");
    const pack = await response.json();
    const base = new URL("./", descriptor);
    let bytes = 0;
    for (const row of pack.files) {
      const url = new URL(row.path, base);
      const result = await fetch(url, { cache: "no-store" });
      if (!result.ok || result.headers.get("X-Moly-Cache") !== "retained")
        throw Error(`Not retained: ${url}`);
      const data = await result.arrayBuffer();
      const hash = [
        ...new Uint8Array(await crypto.subtle.digest("SHA-256", data)),
      ]
        .map((byte) => byte.toString(16).padStart(2, "0"))
        .join("");
      if (data.byteLength !== row.decodedBytes || hash !== row.sha256)
        throw Error(`Source mismatch: ${url}`);
      bytes += data.byteLength;
    }
    return {
      files: pack.files.length,
      bytes,
      source: { region: pack.region, version: pack.gameVersion },
      sha256Verified: true,
    };
  }, descriptor);
}
try {
  const manifest = await fetch(`${options.origin}/moly/manifest.json`).then(
    (response) => response.json(),
  );
  const snapshot = manifest.snapshots.find(
    (row) => row.region === region && row.available,
  );
  assert.ok(snapshot);
  const descriptor = new URL(
    "browser-base.json",
    new URL(snapshot.assets, options.origin),
  ).href;
  record("release", { release: manifest.release.id, snapshot: snapshot.id });
  await page.goto(entry, { waitUntil: "domcontentloaded" });
  await prepared();
  const cold = await cacheCommand();
  assert.equal(cold.enabled, true);
  assert.ok(cold.bytes > 0 && cold.entries > 0);
  record("automatic-cold-cache", cold);
  await context.setOffline(true);
  record("base-offline", await verifyBaseOffline(descriptor));
  await context.setOffline(false);
  // Content-specific models/voice are retained only after actual use.
  await play();
  const expected = await frame.evaluate(
    () => JSON.parse(window.__qa.library_snapshot()).selected.lines,
  );
  assert.ok(expected.length, "Source dialogue is empty");
  record("online-complete-dialogue", await completeDialogue(expected));
  record("warmed-dialogue", await cacheCommand());
  await context.setOffline(true);
  await frame.goto(frame.url(), { waitUntil: "domcontentloaded" });
  await prepared();
  await play();
  await page.screenshot({
    path: path.join(out, `cache-${region}-offline-playing.png`),
  });
  record("offline-dialogue", {
    key,
    transcript: true,
    freshRuntime: true,
    ...(await completeDialogue(expected)),
  });
  await context.setOffline(false);
  const resources = page
    .locator('a[href*="/mysekai/interactions/resources/"]')
    .first();
  const resourcesURL = new URL(
    await resources.getAttribute("href"),
    options.origin,
  );
  assert.equal(resourcesURL.searchParams.get("region"), region);
  assert.equal(resourcesURL.searchParams.get("snapshot"), snapshot.id);
  await resources.click();
  await page.locator("[data-moly-cache-bytes]").waitFor();
  const storageBefore = await page.evaluate(async () => {
    await (
      await caches.open("qa-unrelated-preservation")
    ).put("/qa-preserved", new Response("keep"));
    return JSON.stringify({ ...localStorage });
  });
  await page
    .locator(".interaction-resource-cache button.interaction-button")
    .click();
  await page.waitForFunction(
    () =>
      document
        .querySelector("[data-moly-cache-bytes]")
        ?.getAttribute("data-moly-cache-bytes") === "0",
  );
  const cleared = await cacheCommand();
  assert.equal(cleared.bytes, 0);
  assert.equal(cleared.entries, 0);
  assert.equal(
    await page.evaluate(() => JSON.stringify({ ...localStorage })),
    storageBefore,
  );
  assert.equal(
    await page.evaluate(async () =>
      (
        await (
          await caches.open("qa-unrelated-preservation")
        ).match("/qa-preserved")
      )?.text(),
    ),
    "keep",
  );
  record("scoped-clear", {
    ...cleared,
    localStorageUnchanged: true,
    unrelatedCachePreserved: true,
  });
  await page.screenshot({
    path: path.join(out, `cache-${region}-cleared.png`),
  });
  // Exercise the visible reload link, preserving the selected source in its URL.
  await page.locator(".interaction-resource-cache a").click();
  assert.equal(new URL(page.url()).searchParams.get("region"), region);
  assert.equal(new URL(page.url()).searchParams.get("snapshot"), snapshot.id);
  record("source-navigation", { locale, region, snapshot: snapshot.id });
  await prepared();
  const reloaded = await cacheCommand();
  assert.ok(reloaded.bytes > 0);
  await context.setOffline(true);
  record("reloaded-offline-base", await verifyBaseOffline(descriptor));
  await context.setOffline(false);
  await page.goto(entry, { waitUntil: "domcontentloaded" });
  await prepared();
  await play();
  record("reloaded-complete-dialogue", await completeDialogue(expected));
  await cacheCommand("retain", false);
  await cacheCommand("clear");
  const disabled = await frame.evaluate(async (descriptor) => {
    const response = await fetch(descriptor, { cache: "no-store" });
    await response.arrayBuffer();
    return {
      status: response.status,
      retained: response.headers.get("X-Moly-Cache"),
    };
  }, descriptor);
  assert.equal(disabled.status, 200);
  assert.equal(disabled.retained, null);
  const disabledState = await cacheCommand();
  assert.equal(disabledState.enabled, false);
  assert.equal(disabledState.bytes, 0);
  record("disabled-network-only", disabledState);
  record("http-failures", httpFailures);
  assert.deepEqual(
    httpFailures.filter((row) =>
      new URL(row.url).pathname.startsWith("/moly/"),
    ),
    [],
  );
  assert.deepEqual(failures, []);
  record("passed", true);
} catch (error) {
  failures.push(String(error));
  console.error(error);
  await page
    .screenshot({
      path: path.join(out, `cache-${region}-failure.png`),
      fullPage: true,
    })
    .catch(() => {});
  process.exitCode = 1;
} finally {
  await fs.writeFile(
    path.join(out, `cache-${region}.json`),
    JSON.stringify(
      { passed: failures.length === 0, events, failures, httpFailures },
      null,
      2,
    ),
  );
  await browser.close();
}
