import test from "node:test";
import assert from "node:assert/strict";
import { loadEngine } from "./engine-loader.mjs";

// A loader run with fake renderer, module and transport. `prepare` is the
// source preflight; `gate` holds it until the test releases it.
function harness({ prepareError = null } = {}) {
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
  };
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
  assert.deepEqual(
    events.find(([kind]) => kind === "instantiate")[1],
    { compiled: 4 },
  );
});

test("a failed source preflight never instantiates the engine", async () => {
  const { run, events, release } = harness({
    prepareError: new Error("source mismatch"),
  });
  release();
  await assert.rejects(run, /source mismatch/);
  assert.ok(!events.some(([kind]) => kind === "instantiate"));
});
