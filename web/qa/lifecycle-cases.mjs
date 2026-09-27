import assert from "node:assert/strict";

/** Real intents and one explicitly injected network fault, using source assets. */
export async function runLifecycle({
  page,
  command,
  snapshot,
  audit,
  checkFatal,
  assertRestored,
  record,
  saveFrame,
  expectedFailures,
}) {
  async function observe(predicate, timeout = 30000) {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      await checkFatal();
      const s = await snapshot(),
        d = await audit();
      if (predicate(s, d)) return { s, d };
      await page.waitForTimeout(125);
    }
    throw Error("Lifecycle condition timed out");
  }
  async function play(key, tab) {
    await command("tab", tab);
    await command("fixture", null);
    await command("query", "");
    await observe((s) => s.tab === tab);
    await command("play", undefined, key);
    return observe(
      (s, d) =>
        s.status.activeKey === key &&
        s.status.phase === "playing" &&
        d.independent?.phase === "Experiencing",
      60000,
    );
  }
  async function restored(phase = "idle") {
    await observe(
      (s, d) =>
        s.status.phase === phase && !s.status.canStop && d.independent === null,
    );
    await page.waitForTimeout(900);
    const d = await audit();
    assertRestored(d);
    return d;
  }

  await command("mode", "current");
  await command("tab", "conversations");
  await command("select", undefined, "talk:general:3912");
  await observe(
    (s) => s.mode === "current" && s.selected?.key === "talk:general:3912",
  );
  assert.equal(
    (await snapshot()).selected.available,
    false,
    "empty current scene must reject absent cast",
  );
  await command("play", undefined, "talk:general:3912");
  record("current-scene-rejected", await restored());
  assert.ok(
    (await snapshot()).status.label,
    "Unavailable must explain the missing scene condition",
  );

  await command("mode", "independent");
  const first = await play("fixture:15", "furniture");
  assert.equal(first.d.gimmick_owners, 1);
  await command("tab", "conversations");
  await observe((s) => s.tab === "conversations");
  const before = (await snapshot()).status;
  assert.equal(before.activeKey, "fixture:15");
  assert.equal(before.phase, "playing");
  await command("play", undefined, "talk:general:3912");
  const replaced = await observe(
    (s, d) =>
      s.status.activeKey === "talk:general:3912" &&
      s.status.phase === "playing" &&
      d.independent?.phase === "Experiencing",
    60000,
  );
  assert.equal(replaced.d.gimmick_owners, 0);
  assert.deepEqual(replaced.d.fixtures, []);
  record("replacement-released-old-owner", replaced.d);
  await saveFrame("replacement");
  await command("stop");
  record("replacement-restored-original", await restored());

  await command("tab", "performances");
  await observe((s) => s.tab === "performances");
  await command("play", undefined, "talk:fixture:6998");
  await observe((s) => s.status.phase === "preparing");
  await command("stop");
  record("cancel-during-prepare", await restored());
  await page.waitForTimeout(1600);
  assertRestored(await audit());

  // Fail a real, not-yet-loaded source controller package, never a fabricated
  // masterdata row. The rest of the world must stay responsive and restore.
  let injected = 0;
  const failure = async (route) => {
    const url = route.request().url();
    expectedFailures.add(url);
    injected++;
    await route.fulfill({
      status: 503,
      contentType: "application/json",
      body: '{"error":"qa_injected_unavailable"}',
    });
  };
  await page.route("**/fixture-gimmick/by-package/*.json", failure);
  await command("tab", "furniture");
  await observe((s) => s.tab === "furniture");
  await command("play", undefined, "fixture:297");
  const recovered = await restored("error");
  assert.equal(injected, 1, "selected source package fault was not exercised");
  await page.unroute("**/fixture-gimmick/by-package/*.json", failure);
  record("missing-content-restored", recovered);

  const resumed = await play("talk:general:3912", "conversations");
  assert.ok(resumed.d.transcript);
  // Escape belongs to the host's fullscreen presentation in stage mode.
  // Stop remains the explicit transport intent that restores the scene.
  await command("stop");
  record("stop-restores-after-error", await restored());
  const counts = await page.evaluate(() => ({
    canvases: document.querySelectorAll("canvas").length,
    renderers: performance.getEntriesByName("moly:rendererReady").length,
    contexts: new Set(window.__molyQA.audioContexts).size,
  }));
  assert.equal(counts.canvases, 1);
  assert.equal(counts.renderers, 1);
  assert.equal(counts.contexts, 1);
  record("single-runtime-through-lifecycle", counts);
}
