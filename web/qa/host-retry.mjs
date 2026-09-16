// Explicit resource-failure -> restored world -> user Retry -> fresh realm.
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
const { chromium } = await import(
  process.env.MOLY_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch({
  headless: true,
  channel: process.env.MOLY_BROWSER_CHANNEL || "chrome",
});
// Disabling service workers here makes the single explicit fault interception
// deterministic; storage behavior has its own real-worker browser acceptance.
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
  serviceWorkers: "block",
});
const page = await context.newPage();
page.setDefaultTimeout(25000);
const events = [],
  errors = [],
  expected = new Set();
let failed = false,
  injected = 0;
const record = (type, data) => {
  events.push({ type, data });
  console.log(type, JSON.stringify(data).slice(0, 2000));
};
page.on("pageerror", (e) => errors.push(String(e)));
page.on("response", (r) => {
  if (
    r.status() >= 400 &&
    new URL(r.url()).origin === origin &&
    !expected.has(r.url())
  )
    errors.push(`${r.status()} ${r.url()}`);
});
await page.addInitScript(() => {
  window.__retryQA = { token: crypto.randomUUID(), last: null };
  addEventListener("message", (e) => {
    if (
      e.origin === location.origin &&
      e.source ===
        document.querySelector(".interaction-runtime iframe")?.contentWindow &&
      e.data?.type === "snapshot" &&
      e.data.source === "moly"
    )
      window.__retryQA.last = e.data.value;
  });
});
const fault = async (route) => {
  injected++;
  expected.add(route.request().url());
  await route.fulfill({
    status: 503,
    contentType: "application/json",
    body: '{"error":"qa_expected_package_unavailable"}',
  });
};
const liveFrame = async () => {
  const h = await page.locator(".interaction-runtime iframe").elementHandle();
  return h.contentFrame();
};
async function enter() {
  await page
    .frameLocator(".interaction-runtime iframe")
    .locator('#stage-start[data-moly-ready="true"]')
    .waitFor({ timeout: 60000 });
  await page.locator('[data-action="play-selected"]').click();
}
try {
  await page.route("**/fixture-gimmick/by-package/*.json", fault);
  await page.goto(
    origin +
      "/en-us/mysekai/interactions/?region=cn&content=fixture:297&tab=furniture",
    { waitUntil: "domcontentloaded", timeout: 60000 },
  );
  await enter();
  const oldFrame = await liveFrame();
  const oldToken = await oldFrame.evaluate(() => window.__retryQA.token);
  await page.waitForFunction(
    () =>
      window.__retryQA.last?.status.phase === "error" &&
      !window.__retryQA.last.status.canStop,
    null,
    { timeout: 60000 },
  );
  const errorState = await page.evaluate(() => window.__retryQA.last);
  assert.deepEqual(errorState.scene.actorUnits, []);
  assert.deepEqual(errorState.scene.fixtureIds, []);
  assert.equal(injected, 1);
  record("failed-and-restored", { oldToken, errorState });
  await page.screenshot({ path: path.join(out, "host-retry-error.png") });
  await page.unroute("**/fixture-gimmick/by-package/*.json", fault);
  await page
    .locator('.interaction-stage-column [role="alert"]')
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await enter();
  const newFrame = await liveFrame();
  const newToken = await newFrame.evaluate(() => window.__retryQA.token);
  assert.notEqual(newToken, oldToken);
  assert.ok(oldFrame.isDetached());
  await page.waitForFunction(
    () =>
      window.__retryQA.last?.status.phase === "playing" &&
      window.__retryQA.last.status.activeKey === "fixture:297",
    null,
    { timeout: 60000 },
  );
  assert.equal(await page.locator(".interaction-runtime iframe").count(), 1);
  assert.equal(new URL(page.url()).searchParams.get("content"), "fixture:297");
  record("retry-played", {
    newToken,
    state: await page.evaluate(() => window.__retryQA.last),
  });
  await page.screenshot({ path: path.join(out, "host-retry-playing.png") });
  await page
    .getByRole("button", { name: "Stop and restore scene", exact: true })
    .first()
    .click();
  await page.waitForFunction(
    () =>
      !window.__retryQA.last.status.canStop &&
      window.__retryQA.last.status.phase === "idle",
    null,
    { timeout: 30000 },
  );
  await page.getByRole("button", { name: "Close player", exact: true }).click();
  await page.waitForFunction(
    () => !document.querySelector(".interaction-runtime iframe"),
  );
  assert.deepEqual(errors, []);
} catch (error) {
  failed = true;
  record("failure", String(error));
  record(
    "body",
    (
      await page
        .locator("body")
        .innerText()
        .catch(() => "")
    ).slice(0, 8000),
  );
  await page
    .screenshot({ path: path.join(out, "host-retry-failure.png") })
    .catch(() => {});
} finally {
  await fs.writeFile(
    path.join(out, "host-retry.json"),
    JSON.stringify(
      { failed, errors, injected, expected: [...expected], events },
      null,
      2,
    ),
  );
  await browser.close();
  if (failed) process.exitCode = 1;
}
