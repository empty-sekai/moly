// Real-browser checks against source assets and the existing Rust runtime.
// No fixtures, actors or script results are synthesized by this harness.
import { parseArgs } from "node:util";
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { stageMessages } from "../stage-locale.mjs";
const { values: options } = parseArgs({
  options: {
    origin: { type: "string", default: "http://127.0.0.1:8017" },
    assets: { type: "string" },
    region: { type: "string", default: "cn" },
    version: { type: "string" },
    backend: { type: "string", default: "webgpu" },
    out: { type: "string" },
    suite: { type: "string", default: "smoke" },
    stage: { type: "string", default: "/stage.html" },
    tag: { type: "string", default: "acceptance" },
    mobile: { type: "boolean", default: false },
    content: { type: "string", multiple: true },
    video: { type: "boolean", default: false },
    orbit: { type: "boolean", default: false },
  },
});
assert.ok(["cn", "jp"].includes(options.region));
assert.ok(["webgpu", "webgl2", "auto-fallback"].includes(options.backend));
assert.ok(["smoke", "matrix", "lifecycle"].includes(options.suite));
if (!options.out)
  throw Error("--out must name an evidence directory outside source");
const output = path.resolve(options.out);
await fs.mkdir(output, { recursive: true });
const assets = new URL(
  options.assets ?? (options.region === "cn" ? "/assets/" : "/assets-jp/"),
  options.origin,
);
assert.equal(assets.origin, new URL(options.origin).origin);
const metaResponse = await fetch(new URL("mysekai-fixtures.json", assets));
if (!metaResponse.ok) throw Error("Snapshot master missing");
const meta = await metaResponse.json();
assert.equal(meta.region, options.region);
const version = options.version ?? meta.gameVersion;
assert.equal(meta.gameVersion, version);
const { chromium } = await import(
  process.env.MOLY_PLAYWRIGHT_MODULE || "playwright"
);
const browser = await chromium.launch({
  headless: true,
  channel: process.env.MOLY_BROWSER_CHANNEL || "chrome",
});
const context = await browser.newContext({
  viewport: options.mobile
    ? { width: 390, height: 700 }
    : { width: 1280, height: 800 },
  isMobile: options.mobile,
  hasTouch: options.mobile,
  deviceScaleFactor: 1,
  ...(options.video
    ? {
        recordVideo: {
          dir: path.join(output, "recordings"),
          size: options.mobile
            ? { width: 390, height: 700 }
            : { width: 1280, height: 800 },
        },
      }
    : {}),
});
const page = await context.newPage();
page.setDefaultTimeout(15000);
const backend =
  options.backend === "auto-fallback" ? "webgl2" : options.backend;
const prefix = `${options.tag}-${options.region}-${options.backend}${options.mobile ? "-mobile" : ""}`;
const started = Date.now(),
  events = [],
  errors = [],
  warnings = [],
  requests = [],
  cases = [];
const expectedFailures = new Set();
const isExpected = (text, url = "") =>
  [...expectedFailures].some(
    (expected) =>
      url === expected ||
      text.includes(new URL(expected).pathname.split("/").at(-1)),
  );
let failed = false,
  baseline = null;
const record = (type, data) => {
  events.push({ ms: Date.now() - started, type, data });
  console.log(type, JSON.stringify(data).slice(0, 1800));
};
page.on("pageerror", (error) => {
  errors.push(String(error));
  record("fatal", String(error));
});
page.on("console", (message) => {
  const text = message.text();
  if (message.type() === "error") {
    if (isExpected(text, message.location().url))
      record("expected-resource-error", text);
    else {
      errors.push(text);
      record("console-error", text);
    }
  } else if (message.type() === "warning" && warnings.length < 300)
    warnings.push(text);
  if (/fixture-(?:talk|gimmick)|content-library|\[audio-source\]/.test(text))
    record("owner-log", text);
});
page.on("request", (request) => requests.push(request.url()));
page.on("response", (response) => {
  if (response.status() >= 400) {
    if (expectedFailures.has(response.url()))
      record("expected-http-error", {
        status: response.status(),
        url: response.url(),
      });
    else {
      errors.push(`${response.status()} ${response.url()}`);
      record("http-error", { status: response.status(), url: response.url() });
    }
  }
});
await page.addInitScript(() => {
  window.__molyQA = {
    fatal: null,
    audioContexts: [],
    audioStarts: [],
    audioFrames: 0,
    audioPeak: 0,
  };
  addEventListener("moly-error", (e) => {
    window.__molyQA.fatal = String(e.detail);
  });
  for (const key of ["AudioContext", "webkitAudioContext"]) {
    const Original = window[key];
    if (Original)
      window[key] = new Proxy(Original, {
        construct(target, args, newTarget) {
          const value = Reflect.construct(target, args, newTarget);
          window.__molyQA.audioContexts.push(value);
          const createBufferSource = value.createBufferSource.bind(value);
          value.createBufferSource = () => {
            const source = createBufferSource();
            const start = source.start.bind(source);
            source.start = (...parameters) => {
              let peak = 0;
              for (
                let channel = 0;
                channel < (source.buffer?.numberOfChannels ?? 0);
                channel++
              ) {
                for (const sample of source.buffer.getChannelData(channel))
                  peak = Math.max(peak, Math.abs(sample));
              }
              window.__molyQA.audioFrames += source.buffer?.length ?? 0;
              window.__molyQA.audioPeak = Math.max(
                window.__molyQA.audioPeak,
                peak,
              );
              if (
                window.__molyQA.audioStarts.length < 16 ||
                (peak > 0.0001 && window.__molyQA.audioStarts.length < 256)
              )
                window.__molyQA.audioStarts.push({
                  frames: source.buffer?.length ?? 0,
                  state: value.state,
                  duration: source.buffer?.duration ?? 0,
                  peak,
                });
              return start(...parameters);
            };
            return source;
          };
          return value;
        },
      });
  }
});
// A capability failure is simulated, but the resulting WebGL renderer is real.
if (options.backend === "auto-fallback")
  await page.addInitScript(() =>
    Object.defineProperty(navigator, "gpu", {
      configurable: true,
      value: { requestAdapter: async () => null },
    }),
  );
const snapshot = () =>
  page.evaluate(() => JSON.parse(window.__molyQA.wasm.library_snapshot()));
const audit = () =>
  page.evaluate(() => JSON.parse(window.__molyQA.wasm.library_diagnostics()));
async function command(type, value, key) {
  await page.evaluate(
    ({ type, value, key }) =>
      window.__molyQA.wasm.library_command(
        JSON.stringify({
          schemaVersion: 1,
          type,
          ...(value !== undefined ? { value } : {}),
          ...(key ? { key } : {}),
        }),
      ),
    { type, value, key },
  );
}
async function checkFatal() {
  const fatal = await page.evaluate(() => window.__molyQA.fatal);
  if (fatal) throw Error(fatal);
  if (errors.length) throw Error(errors.at(-1));
}
async function until(predicate, timeout = 60000) {
  const begin = Date.now();
  while (Date.now() - begin < timeout) {
    await checkFatal();
    const s = await snapshot(),
      d = await audit();
    if (s.status.phase === "error")
      throw Error(s.status.error || "Playback error");
    if (predicate(s, d)) return { s, d };
    await page.waitForTimeout(125);
  }
  throw Error("Acceptance condition not met within fixed budget");
}
function approx(actual, expected, tolerance, label) {
  assert.equal(actual.length, expected.length, label);
  for (let i = 0; i < actual.length; i++)
    assert.ok(
      Math.abs(actual[i] - expected[i]) <= tolerance,
      `${label}[${i}]: ${actual[i]} vs ${expected[i]}`,
    );
}
function assertRestored(d) {
  assert.equal(d.site, baseline.site, "original site");
  assert.equal(d.independent, null, "temporary session");
  assert.equal(d.scene_input_owned, false, "scene owner");
  assert.equal(d.scene_preview, false, "placement owner");
  assert.equal(d.gimmick_owners, 0, "gimmick leases");
  assert.equal(d.player_control_owned, false, "player controller");
  assert.equal(d.player_fixture_active, false, "fixture runtime");
  assert.equal(d.general_owner, null, "general owner");
  assert.equal(d.fixture_owner, null, "fixture talk owner");
  assert.equal(d.activity?.active ?? false, false, "activity owner");
  assert.equal(d.activity_bubbles, 0, "activity bubbles");
  assert.equal(d.held_actors, baseline.held_actors, "actor holds");
  assert.equal(d.voice_players, baseline.voice_players, "voice players");
  assert.equal(d.scoped_sounds, baseline.scoped_sounds, "scoped sounds");
  assert.deepEqual(
    d.fixtures.map((f) => f.id).sort((a, b) => a - b),
    baseline.fixtures.map((f) => f.id).sort((a, b) => a - b),
    "fixture layout",
  );
  assert.deepEqual(
    d.actors.map((a) => a.unit).sort((a, b) => a - b),
    baseline.actors.map((a) => a.unit).sort((a, b) => a - b),
    "cast",
  );
  approx(d.player.position, baseline.player.position, 0.002, "player position");
  approx(d.player.rotation, baseline.player.rotation, 0.002, "player rotation");
  assert.equal(d.camera.state, baseline.camera.state, "camera state");
  for (const field of ["yaw", "pitch", "distance", "gesturedDistance", "fov"])
    approx(
      [d.camera.model[field]],
      [baseline.camera.model[field]],
      0.003,
      `camera.${field}`,
    );
  assert.equal(
    d.camera.views.length,
    baseline.camera.views.length,
    "renderer count",
  );
  for (let i = 0; i < d.camera.views.length; i++) {
    approx(
      [d.camera.views[i].fov],
      [baseline.camera.views[i].fov],
      0.0001,
      "lens",
    );
    approx(
      d.camera.views[i].position,
      baseline.camera.views[i].position,
      0.03,
      "camera position",
    );
  }
  assert.equal(d.room_appearance.wall, baseline.room_appearance.wall, "wall");
  assert.equal(
    d.room_appearance.floor,
    baseline.room_appearance.floor,
    "floor",
  );
  assert.equal(
    d.harvest_nodes,
    baseline.harvest_nodes,
    "original gathering population",
  );
}
async function saveFrame(name) {
  await page.screenshot({
    path: path.join(output, `${prefix}-${name}.png`),
    timeout: 15000,
  });
}
try {
  const locale = options.region === "jp" ? "ja-JP" : "en-US";
  const url = new URL(options.stage, options.origin);
  for (const [key, value] of Object.entries({
    region: options.region,
    version,
    assets: assets.pathname,
    locale,
    preload: "1",
    renderer: options.backend === "auto-fallback" ? "auto" : backend,
  }))
    url.searchParams.set(key, value);
  await page.goto(url.href, { waitUntil: "domcontentloaded", timeout: 60000 });
  await page
    .locator("#stage-start")
    .filter({ hasText: stageMessages(locale).enter })
    .waitFor({ timeout: 60000 });
  await page.locator("#stage-start").click();
  await page.evaluate(
    async (module) => {
      window.__molyQA.wasm = await import(module);
    },
    new URL(`./pkg/${backend}/moly-app.js`, url).href,
  );
  await until(
    (s, d) => s.ready && s.scene?.ready && d.ready && d.readiness.site_scenes,
  );
  await page.waitForTimeout(1000);
  baseline = await audit();
  assert.deepEqual((await snapshot()).scene.actorUnits, []);
  assert.deepEqual((await snapshot()).scene.fixtureIds, []);
  record("baseline", baseline);
  record(
    "base-resources",
    await page.evaluate(() =>
      performance.getEntriesByType("resource").map((entry) => ({
        name: entry.name,
        decodedBytes: entry.decodedBodySize,
      })),
    ),
  );
  await page.waitForFunction(
    () => {
      const c = JSON.parse(window.__molyQA.wasm.library_catalog());
      if (c.ready) {
        window.__molyQA.catalog = c;
        return true;
      }
      return false;
    },
    null,
    { timeout: 30000, polling: 250 },
  );
  const catalog = await page.evaluate(() => window.__molyQA.catalog);
  assert.equal(catalog.region, options.region);
  assert.equal(catalog.version, version);
  await fs.writeFile(
    path.join(output, `catalog-${options.region}.json`),
    JSON.stringify(catalog),
  );
  record("catalog", {
    entries: catalog.entries.length,
    region: catalog.region,
    version,
  });
  if (options.suite === "lifecycle") {
    const { runLifecycle } = await import("./lifecycle-cases.mjs");
    await runLifecycle({
      page,
      command,
      snapshot,
      audit,
      checkFatal,
      assertRestored,
      record,
      saveFrame,
      expectedFailures,
    });
  }
  const definitions =
    options.suite === "lifecycle"
      ? []
      : options.content?.length
        ? options.content.map((key) => {
            const row = catalog.entries.find((entry) => entry.key === key);
            return [
              key,
              key.startsWith("talk:") ||
              row?.presentation.behavior === "one_shot"
                ? "natural"
                : "hold",
            ];
          })
        : options.suite === "matrix"
          ? [
              ["talk:general:3912", "natural"],
              ["talk:fixture:261", "natural"],
              ["talk:fixture:6998", "natural"],
              ["talk:fixture:5657", "natural"],
              ["fixture:7", "inspect"],
              ["fixture:15", "hold"],
              ["fixture:8", "hold"],
              ["fixture:297", "natural"],
              ["activity:notalk:1:1", "hold"],
              ["activity:preaction:258:14", "hold"],
              ["activity:preaction:6791:272", "hold"],
            ]
          : [["talk:general:3912", "natural"]];
  for (const [key, finish] of definitions) {
    const entry = catalog.entries.find((e) => e.key === key);
    assert.ok(entry?.available, `Exact source sample unavailable: ${key}`);
    const one = {
      key,
      title: entry.title,
      presentation: entry.presentation,
      pass: false,
      samples: [],
      firstRequest: requests.length,
    };
    cases.push(one);
    record("case", key);
    const tab =
      key === "talk:general:3912"
        ? "conversations"
        : key.startsWith("fixture:")
          ? "furniture"
          : key.startsWith("activity:")
            ? "activities"
            : "performances";
    await command("tab", tab);
    await command("fixture", null);
    await command("query", "");
    await command("availability", "all");
    await until((s) => s.tab === tab);
    await command("play", undefined, key);
    await until(
      (s) => s.status.phase === "playing" && s.status.activeKey === key,
    );
    const begin = Date.now();
    let lastFrame = -1,
      lastAdvance = 0,
      ended = false,
      orbited = false;
    for (let tick = 0; tick < 260; tick++) {
      await checkFatal();
      const s = await snapshot(),
        d = await audit(),
        elapsed = (Date.now() - begin) / 1000;
      if (options.mobile && d.transcript.visible && elapsed > 1) {
        const layout = d.dialogue_layout;
        assert.ok(
          layout && layout.font_px > 0,
          "mobile dialogue has no source canvas metrics",
        );
        // Source panel width is 1600 and authored text size is 40. Preserve
        // their ratio instead of reintroducing the rejected responsive redraw.
        assert.ok(
          Math.abs(layout.font_px - (layout.max[0] - layout.min[0]) / 40) < 0.1,
          "mobile dialogue changed the authored panel/text scale ratio",
        );
        const viewport = page.viewportSize();
        assert.ok(
          layout.min[0] >= -viewport.width / 2 &&
            layout.max[0] <= viewport.width / 2,
          "dialogue horizontal bounds",
        );
        assert.ok(
          layout.min[1] >= -viewport.height / 2 &&
            layout.max[1] <= viewport.height / 2,
          "dialogue vertical bounds",
        );
      }
      one.samples.push({
        seconds: elapsed,
        phase: s.status.phase,
        transcript: d.transcript,
        activity: d.activity,
        activityBubbles: d.activity_bubbles,
        voices: d.voice_players,
        sounds: d.scoped_sounds,
        gimmicks: d.gimmick_owners,
        camera: d.camera,
        player: d.player,
        actors: d.actors,
        fixtures: d.fixtures,
      });
      if (s.status.phase === "error")
        throw Error(`Runtime rejected ${key}: ${s.status.error}`);
      if (d.independent) {
        assert.equal(d.site, "home_site");
        assert.equal(
          d.harvest_nodes,
          0,
          "clean preview has no gathering objects",
        );
        assert.deepEqual(d.independent.required_fixtures, [
          ...entry.fixtureIds,
        ]);
      }
      const frameAt =
        elapsed >= 12 ? 12 : elapsed >= 6 ? 6 : elapsed >= 2 ? 2 : 0;
      if (frameAt !== lastFrame) {
        lastFrame = frameAt;
        await saveFrame(`${key.replaceAll(":", "-")}-${frameAt}s`);
      }
      if (s.status.phase === "idle" && !s.status.canStop) {
        ended = true;
        break;
      }
      if (
        finish === "natural" &&
        d.transcript.visible &&
        elapsed - lastAdvance > 1.6
      ) {
        const size = page.viewportSize();
        await page.mouse.click(size.width * 0.82, size.height * 0.83);
        lastAdvance = elapsed;
      }
      if (options.orbit && !orbited && elapsed >= 7) {
        orbited = true;
        const size = page.viewportSize();
        await page.mouse.move(size.width * 0.65, size.height * 0.38);
        await page.mouse.down();
        await page.mouse.move(size.width * 0.3, size.height * 0.42, {
          steps: 30,
        });
        await page.mouse.up();
      }
      if (finish !== "natural" && elapsed >= 15) {
        await command("stop");
        break;
      }
      if (elapsed >= 65) throw Error(`Content did not complete: ${key}`);
      await page.waitForTimeout(250);
    }
    await until(
      (s, d) =>
        s.status.phase === "idle" &&
        !s.status.canStop &&
        !d.independent &&
        !d.scene_input_owned,
      30000,
    );
    await page.waitForTimeout(800);
    one.restored = await audit();
    assertRestored(one.restored);
    one.requests = requests.slice(one.firstRequest);
    assert.ok(
      !one.requests.some((u) => u.endsWith("/fixture-gimmick/gimmicks.json")),
    );
    if (finish === "natural")
      assert.ok(ended, `Natural completion not observed: ${key}`);
    if (key.startsWith("talk:")) {
      const shown = one.samples.filter(
        (sample) =>
          sample.transcript?.visible && sample.transcript.text?.trim(),
      );
      assert.ok(shown.length, `No visible source dialogue: ${key}`);
      for (const line of entry.lines)
        assert.ok(
          shown.some(
            (sample) =>
              sample.transcript.text === line.text &&
              sample.transcript.speaker === line.speaker,
          ),
          `Source dialogue line not shown: ${key} ${JSON.stringify(line)}`,
        );
    }
    if (key === "activity:preaction:258:14")
      assert.ok(
        one.samples.some((s) => s.activityBubbles > 0),
        "source bubble did not appear",
      );
    if (key === "talk:fixture:5657")
      assert.ok(
        one.samples.some((s) => s.gimmicks > 0),
        "authored loop never obtained a lease",
      );
    one.pass = true;
    record("case-passed", {
      key,
      natural: ended,
      frames: lastFrame,
      peakGimmicks: Math.max(...one.samples.map((s) => s.gimmicks)),
    });
    await fs.writeFile(
      path.join(output, `${prefix}-progress.json`),
      JSON.stringify({ baseline, cases }, null, 2),
    );
  }
  record(
    "resource-metrics",
    await page.evaluate(() => ({
      marks: performance
        .getEntriesByType("mark")
        .filter((e) => e.name.startsWith("moly:"))
        .map((e) => ({ name: e.name, time: e.startTime })),
      resources: performance.getEntriesByType("resource").map((e) => ({
        name: e.name,
        transfer: e.transferSize,
        decoded: e.decodedBodySize,
        duration: e.duration,
      })),
    })),
  );
  await checkFatal();
  const audio = await page.evaluate(() => ({
    contexts: window.__molyQA.audioContexts.map((context) => context.state),
    starts: window.__molyQA.audioStarts,
    frames: window.__molyQA.audioFrames,
    peak: window.__molyQA.audioPeak,
  }));
  assert.equal(audio.contexts.length, 1, "playback must own one AudioContext");
  assert.equal(
    audio.contexts[0],
    "running",
    "trusted gesture must activate audio",
  );
  assert.ok(
    audio.starts.some(
      (source) =>
        source.frames > 0 && source.state === "running" && source.peak > 0.0001,
    ),
    "non-silent PCM must reach the running audio context",
  );
  record("audio-verified", audio);
} catch (error) {
  failed = true;
  record("failure", String(error));
  record("last-audit", await audit().catch(() => null));
  await saveFrame("failure").catch(() => {});
} finally {
  const video = page.video();
  await context.close();
  if (video) {
    await video.saveAs(path.join(output, `${prefix}.webm`));
    record("video", `${prefix}.webm`);
  }
  await fs.writeFile(
    path.join(output, `${prefix}.json`),
    JSON.stringify(
      {
        schemaVersion: 1,
        failed,
        options,
        baseline,
        cases,
        errors,
        warnings,
        requests,
        events,
      },
      null,
      2,
    ),
  );
  await browser.close();
  if (failed) process.exitCode = 1;
}
