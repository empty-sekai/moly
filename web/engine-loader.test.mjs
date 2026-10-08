import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { loadEngine, fetchUiFont, installUiFont } from "./engine-loader.mjs";

// A loader run with fake renderer, module and transport. `prepare` is the
// source preflight; `gate` holds it until the test releases it.
const fontBytes = new TextEncoder().encode("font bytes");
const fontDigest = createHash("sha256").update(fontBytes).digest("hex");

function harness({ prepareError = null, font = true } = {}) {
  const events = [];
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const module = {
    default: async ({ module_or_path }) => {
      events.push(["instantiate", module_or_path]);
    },
    start_game() {},
    install_ui_font(bytes) {
      events.push(["font", new TextDecoder().decode(bytes)]);
    },
  };
  if (!font) delete module.install_ui_font;
  const run = loadEngine({
    environment: {},
    contract: "game",
    exports: ["start_game"],
    chooseRenderer: async () => ({ backend: "webgl2" }),
    resolve: (backend) =>
      new URL(`https://cdn.test/releases/r/pkg/${backend}/moly-app.js`),
    importModule: async (href) => {
      events.push(["import", href]);
      return module;
    },
    fetchImpl: async (url, options) => {
      events.push(["fetch", String(url)]);
      if (String(url).endsWith("/build.json"))
        return Response.json(
          font
            ? {
                uiFont: {
                  path: `fonts/${fontDigest}.ttf`,
                  sha256: fontDigest,
                  bytes: fontBytes.length,
                },
              }
            : {},
        );
      if (String(url).endsWith(".ttf")) return new Response(fontBytes);
      return new Response(new Uint8Array([0, 97, 115, 109]), {
        headers: { "content-type": "application/wasm" },
      });
    },
    compileModule: async (response) => {
      const bytes = new Uint8Array(await response.arrayBuffer());
      events.push(["compiled", bytes.length]);
      return { compiled: bytes.length };
    },
    async prepare({ signal }) {
      events.push(["prepare"]);
      await gate;
      if (prepareError) throw prepareError;
      events.push(["prepared"]);
      return { ready: Promise.resolve() };
    },
    mark: (name) => events.push(["mark", name]),
  });
  return { run, events, release };
}

test("the engine downloads and compiles while the source preflight runs", async () => {
  const { run, events, release } = harness();
  while (!events.some(([kind]) => kind === "compiled"))
    await new Promise((resolve) => setTimeout(resolve, 1));
  assert.ok(!events.some(([kind]) => kind === "prepared"));
  assert.ok(!events.some(([kind]) => kind === "instantiate"));
  release();
  const { backend } = await run;
  assert.equal(backend, "webgl2");
  const order = events.map(([kind, value]) => (kind === "mark" ? value : kind));
  assert.ok(order.indexOf("prepared") < order.indexOf("instantiate"));
  assert.ok(order.indexOf("instantiate") < order.indexOf("wasmReady"));
  assert.deepEqual(events.find(([kind]) => kind === "instantiate")[1], {
    compiled: 4,
  });
  // The font is read from beside the engine and installed before any start.
  assert.ok(
    events.some(
      ([kind, value]) =>
        kind === "fetch" &&
        value === `https://cdn.test/releases/r/pkg/fonts/${fontDigest}.ttf`,
    ),
  );
  assert.ok(order.indexOf("instantiate") < order.indexOf("font"));
  assert.ok(order.indexOf("font") < order.indexOf("wasmReady"));
});

test("an engine whose build record names no font installs none", async () => {
  const { run, events, release } = harness({ font: false });
  release();
  await run;
  assert.ok(!events.some(([kind]) => kind === "font"));
});

test("a font that differs from its record is refused", async () => {
  const engine = new URL("https://cdn.test/releases/r/pkg/webgl2/moly-app.js");
  const record = {
    uiFont: {
      path: `fonts/${fontDigest}.ttf`,
      sha256: fontDigest,
      bytes: fontBytes.length,
    },
  };
  const serve = (body) => async (url) =>
    String(url).endsWith("/build.json")
      ? Response.json(record)
      : new Response(body);
  assert.deepEqual(
    await fetchUiFont(engine, { fetchImpl: serve(fontBytes) }),
    fontBytes,
  );
  const changed = fontBytes.slice();
  changed[0] ^= 1;
  await assert.rejects(
    fetchUiFont(engine, { fetchImpl: serve(changed) }),
    /checksum mismatch/,
  );
  await assert.rejects(
    fetchUiFont(engine, { fetchImpl: serve(fontBytes.slice(1)) }),
    /length mismatch/,
  );
  await assert.rejects(
    fetchUiFont(engine, {
      fetchImpl: async () =>
        Response.json({ uiFont: { ...record.uiFont, path: "../x.ttf" } }),
    }),
    /Invalid UI font record/,
  );
  for (const uiFont of [
    null,
    [],
    false,
    { ...record.uiFont, bytes: 0 },
    { ...record.uiFont, bytes: 64 * 1024 * 1024 + 1 },
    { ...record.uiFont, sha256: "ab".repeat(32) },
  ]) {
    await assert.rejects(
      fetchUiFont(engine, {
        fetchImpl: async () => Response.json({ uiFont }),
      }),
      /Invalid UI font record/,
    );
  }
  await assert.rejects(
    fetchUiFont(engine, {
      fetchImpl: serve(new Uint8Array(fontBytes.length + 1)),
    }),
    /exceeds its recorded length/,
  );
});

test("font metadata and installation ABI must agree", () => {
  assert.throws(
    () => installUiFont({ install_ui_font() {} }, null),
    /record is missing/,
  );
  assert.throws(() => installUiFont({}, fontBytes), /cannot install/);
  assert.doesNotThrow(() => installUiFont({}, null));
});

test("a failed font read aborts an engine download before instantiation", async () => {
  let instantiated = false,
    aborted = false;
  await assert.rejects(
    loadEngine({
      chooseRenderer: async () => ({ backend: "webgl2" }),
      resolve: () => new URL("https://cdn.test/pkg/webgl2/moly-app.js"),
      importModule: async () => ({
        default() {
          instantiated = true;
        },
      }),
      fetchImpl: async (url, { signal }) => {
        if (String(url).endsWith("build.json"))
          return new Response("missing", { status: 404 });
        return new Promise((resolve, reject) => {
          const stop = () => {
            aborted = true;
            reject(signal.reason);
          };
          if (signal.aborted) stop();
          else signal.addEventListener("abort", stop, { once: true });
        });
      },
    }),
    /build record unavailable/,
  );
  assert.equal(aborted, true);
  assert.equal(instantiated, false);
});

test("a failed source preflight never instantiates the engine", async () => {
  const { run, events, release } = harness({
    prepareError: new Error("source mismatch"),
  });
  release();
  await assert.rejects(run, /source mismatch/);
  assert.ok(!events.some(([kind]) => kind === "instantiate"));
});
