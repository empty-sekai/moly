// End-to-end product path through the real Moesekai host and Moly frame.
import { parseArgs } from "node:util";
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
const { values } = parseArgs({
  options: {
    origin: { type: "string" },
    out: { type: "string" },
    mobile: { type: "boolean", default: false },
    tag: { type: "string", default: "host" },
  },
});
if (!values.origin || !values.out)
  throw Error("Usage: --origin http://host --out /outside/source/evidence");
const origin = new URL(values.origin).origin,
  out = path.resolve(values.out);
await fs.mkdir(out, { recursive: true });
const manifest = await fetch(origin + "/moly/manifest.json").then((r) => {
  if (!r.ok) throw Error("Deployment unavailable");
  return r.json();
});
const source = manifest.snapshots.find((s) => s.region === "cn" && s.available);
assert.ok(source, "CN snapshot unavailable");
const { chromium } = await import(
  process.env.MOLY_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch({
  headless: true,
  channel: process.env.MOLY_BROWSER_CHANNEL || "chrome",
});
const context = await browser.newContext({
  viewport: values.mobile
    ? { width: 390, height: 844 }
    : { width: 1440, height: 1000 },
  isMobile: values.mobile,
  hasTouch: values.mobile,
  colorScheme: "light",
  permissions: ["clipboard-read", "clipboard-write"],
});
const page = await context.newPage();
page.setDefaultTimeout(25000);
const events = [],
  errors = [],
  requests = [],
  start = Date.now();
let failed = false;
const record = (type, data) => {
  events.push({ ms: Date.now() - start, type, data });
  console.log(type, JSON.stringify(data).slice(0, 1600));
};
page.on("pageerror", (e) => {
  errors.push(String(e));
  record("fatal", String(e));
});
page.on("response", (r) => {
  if (r.status() >= 400 && new URL(r.url()).origin === origin) {
    errors.push(`${r.status()} ${r.url()}`);
    record("http-error", { status: r.status(), url: r.url() });
  }
});
page.on("request", (r) =>
  requests.push({ ms: Date.now() - start, url: r.url() }),
);
await page.exposeFunction("__hostQAEvent", record);
await page.addInitScript(() => {
  window.__hostQA = {
    token: crypto.randomUUID(),
    last: null,
    closed: [],
    audio: [],
  };
  for (const key of ["AudioContext", "webkitAudioContext"]) {
    const Original = window[key];
    if (Original)
      window[key] = new Proxy(Original, {
        construct(target, args, newTarget) {
          const value = Reflect.construct(target, args, newTarget);
          window.__hostQA.audio.push(value);
          return value;
        },
      });
  }
  addEventListener("message", (e) => {
    if (
      e.origin !== location.origin ||
      e.source !==
        document.querySelector(".interaction-runtime iframe")?.contentWindow ||
      e.data?.source !== "moly"
    )
      return;
    if (e.data.type === "snapshot") window.__hostQA.last = e.data.value;
    if (e.data.type === "closed") {
      window.__hostQA.closed.push(e.data.value);
      void window.__hostQAEvent("restore-ack", e.data.value);
    }
  });
});
const save = async (name) => {
  await page.screenshot({
    path: path.join(
      out,
      `${values.tag}${values.mobile ? "-mobile" : ""}-${name}.png`,
    ),
    timeout: 15000,
  });
};
async function state() {
  return page.evaluate(() => window.__hostQA.last);
}
async function playing(key) {
  await page.waitForFunction(
    (key) =>
      window.__hostQA.last?.status.phase === "playing" &&
      window.__hostQA.last.status.activeKey === key,
    key,
    { timeout: 60000 },
  );
  const view = await frame();
  await view.evaluate(async () => {
    const module = performance
      .getEntriesByType("resource")
      .find((row) => /moly-app\.js$/.test(row.name));
    window.__hostQA.wasm = await import(module.name);
  });
  await view.waitForFunction(
    (key) => {
      const state = JSON.parse(window.__hostQA.wasm.library_snapshot());
      const diagnostics = JSON.parse(
        window.__hostQA.wasm.library_diagnostics(),
      );
      return (
        state.scene?.ready &&
        (!key.startsWith("talk:") ||
          (diagnostics.transcript?.visible &&
            diagnostics.transcript.text?.trim()))
      );
    },
    key,
    { timeout: 60000 },
  );
}
async function frame() {
  const handle = await page
    .locator(".interaction-runtime iframe")
    .elementHandle();
  return handle?.contentFrame();
}
async function realm() {
  const f = await frame();
  return {
    host: await page.evaluate(() => window.__hostQA.token),
    frame: await f.evaluate(() => window.__hostQA.token),
    audio: await f.evaluate(() => window.__hostQA.audio.length),
    renderers: await f.evaluate(
      () => performance.getEntriesByName("moly:rendererReady").length,
    ),
    session: await page
      .locator(".interaction-runtime")
      .getAttribute("data-moly-session"),
  };
}
async function setLanguage(from, to) {
  await page.locator('button[title*="⌘X"]').first().click();
  await page
    .locator('button[aria-haspopup="listbox"]')
    .filter({ hasText: from })
    .click();
  await page.getByRole("option", { name: to, exact: true }).click();
  await page.keyboard.press("Escape");
}
try {
  await page.goto(origin + "/en-us/mysekai/157/?region=cn", {
    waitUntil: "domcontentloaded",
    timeout: 60000,
  });
  const entry = page.locator('[data-mysekai-interactions-entry="157"]');
  await entry.waitFor({ timeout: 60000 });
  const href = await entry.getAttribute("href");
  assert.equal(new URL(href, origin).searchParams.get("fixture"), "157");
  assert.equal(new URL(href, origin).searchParams.get("region"), "cn");
  assert.ok(!requests.some((r) => /\.wasm(?:\?|$)/.test(r.url)));
  await save("furniture-entry");
  record("furniture-entry", {
    href,
    heading: await page.locator("h1").first().innerText(),
  });
  await entry.click();
  await page.locator('[data-selected-content="fixture:157"]').waitFor();
  await page.waitForFunction(
    (snapshot) =>
      new URL(location.href).searchParams.get("snapshot") === snapshot,
    source.id,
  );
  assert.equal(new URL(page.url()).searchParams.get("fixture"), "157");
  assert.equal(new URL(page.url()).searchParams.get("snapshot"), source.id);
  await save("context-preparing");
  await page
    .frameLocator(".interaction-runtime iframe")
    .locator('#stage-start[data-moly-ready="true"]')
    .waitFor({ timeout: 60000 });
  await page.locator('[data-action="play-selected"]').click();
  await playing("fixture:157");
  const original = await realm();
  assert.equal(original.audio, 1);
  assert.equal(original.renderers, 1);
  await save("fixture-playing");
  await page.locator('[data-tab="performances"]').click();
  await page.waitForTimeout(300);
  assert.equal((await state()).status.activeKey, "fixture:157");
  assert.equal((await state()).status.phase, "playing");
  await page.locator('[data-content-key="talk:fixture:261"]').click();
  assert.equal((await state()).status.activeKey, "fixture:157");
  await page.locator('[data-action="play-selected"]').click();
  await playing("talk:fixture:261");
  await page.waitForTimeout(1600);
  assert.deepEqual(await realm(), original);
  await save("fridge-conversation");
  record("replace-same-world", await realm());
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForTimeout(500);
  assert.deepEqual(await realm(), original);
  await save("dark-playing");
  if (!values.mobile) {
    await setLanguage("English", "日本語");
    await page.waitForURL("**/ja-jp/mysekai/interactions/**");
    await page.waitForTimeout(600);
    assert.deepEqual(await realm(), original);
    assert.equal((await state()).status.activeKey, "talk:fixture:261");
    assert.equal(
      await (await frame()).locator("html").getAttribute("lang"),
      "ja-JP",
    );
    await save("japanese-live");
    await setLanguage("日本語", "English");
    await page.waitForURL("**/en-us/mysekai/interactions/**");
    await page.waitForTimeout(500);
    assert.deepEqual(await realm(), original);
  }
  await page
    .locator(".interaction-detail")
    .getByRole("button", { name: "Share this moment", exact: true })
    .click();
  const shared = await page.evaluate(() => navigator.clipboard.readText());
  const shareURL = new URL(shared);
  assert.equal(shareURL.searchParams.get("content"), "talk:fixture:261");
  assert.equal(shareURL.searchParams.get("snapshot"), source.id);
  assert.equal(shareURL.searchParams.get("fixture"), "157");
  record("share", shared);
  const beforeLeave = events.length;
  await page
    .locator(".interaction-fixture-link")
    .filter({ hasText: "冰箱" })
    .click();
  await page.waitForURL("**/mysekai/157/**", { timeout: 30000 });
  assert.ok(
    events
      .slice(beforeLeave)
      .some((e) => e.type === "restore-ack" && e.data.restored === true),
  );
  assert.equal(await page.locator(".interaction-runtime iframe").count(), 0);
  assert.equal(new URL(page.url()).searchParams.get("region"), "cn");
  record("returned-after-restore", page.url());
  await page.goBack({ waitUntil: "domcontentloaded" });
  await page.locator('[data-selected-content="talk:fixture:261"]').waitFor();
  assert.equal(new URL(page.url()).searchParams.get("fixture"), "157");
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.locator('[data-selected-content="talk:fixture:261"]').waitFor();
  await page
    .frameLocator(".interaction-runtime iframe")
    .locator('#stage-start[data-moly-ready="true"]')
    .waitFor({ timeout: 60000 });
  assert.ok(!(await state())?.status?.canStop, "refresh cannot autoplay");
  assert.equal(new URL(page.url()).searchParams.get("snapshot"), source.id);
  await save("refreshed-share");
  assert.deepEqual(errors, []);
  record("pass", { requests: requests.length, realm: original });
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
    ).slice(0, 10000),
  );
  await save("failure").catch(() => {});
} finally {
  await fs.writeFile(
    path.join(out, `${values.tag}${values.mobile ? "-mobile" : ""}.json`),
    JSON.stringify(
      { failed, origin, manifest, events, errors, requests },
      null,
      2,
    ),
  );
  await browser.close();
  if (failed) process.exitCode = 1;
}
