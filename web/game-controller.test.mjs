import test from "node:test";
import assert from "node:assert/strict";
import {
  GAME_DOCUMENTS,
  GAME_EXPORTS,
  createGameController,
  gameCommand,
  isGameSnapshot,
  parsePersist,
} from "./game-controller.mjs";

// The wire forms of game ABI 2, as the engine publishes and accepts them.
const SNAPSHOT = {
  schemaVersion: 1,
  abi: 2,
  entry: { cover: "opaque", step: "Title", controlOpen: false, hudOpen: false },
  writable: true,
  persist: { revision: 7, acked: 6 },
  server: { revision: 3 },
  memory: { wasmBytes: 123456789 },
  errors: ["refused"],
};

test("game commands serialize to the ABI 2 wire forms and nothing else", () => {
  for (const value of ["hidden", "visible", "pagehide"])
    assert.deepEqual(gameCommand("lifecycle", value), { type: "lifecycle", value });
  assert.deepEqual(gameCommand("input", true), { type: "input", captured: true });
  assert.deepEqual(gameCommand("persist.ack", 7), { type: "persist.ack", revision: 7 });
  assert.deepEqual(
    gameCommand("server.edit", { path: "userMysekaiStamina.normalStamina", value: 10 }),
    { type: "server.edit", path: "userMysekaiStamina.normalStamina", value: 10 },
  );
  assert.deepEqual(gameCommand("server.edit", { action: "sync" }), { type: "server.edit", action: "sync" });
  for (const [type, value] of [
    ["lifecycle", "frozen"],
    ["input", 1],
    ["persist.ack", -1],
    ["site", "home_site"],
    ["server.edit", { path: "x" }],
    ["server.edit", { action: "teleport" }],
    ["server.edit", { type: "lifecycle", path: "x", value: 1 }],
    ["server.edit", { path: "x", value: 1, extra: true }],
  ])
    assert.throws(() => gameCommand(type, value), /Unknown game command/);
});

test("the ABI 2 snapshot and persist forms are admitted; other versions are not", () => {
  assert.equal(isGameSnapshot(SNAPSHOT), true);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, server: { revision: null } }), true);
  for (const cover of ["fading", "gone"])
    assert.equal(isGameSnapshot({ ...SNAPSHOT, entry: { ...SNAPSHOT.entry, cover } }), true);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, abi: 1 }), false);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, schemaVersion: 2 }), false);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, server: undefined }), false);
  assert.equal(parsePersist("", 7), null);
  assert.deepEqual(
    parsePersist('{"revision":8,"documents":{"settings":"{}","server":null,"local":"{}"}}', 7),
    { revision: 8, documents: { settings: "{}", server: null, local: "{}" } },
  );
  assert.throws(() => parsePersist('{"revision":8,"documents":{"settings":"{}"}}', 7));
  assert.throws(() => parsePersist('{"revision":7,"documents":{"settings":"{}","server":null,"local":null}}', 7));
  assert.deepEqual(GAME_DOCUMENTS, ["settings", "server", "local"]);
  assert.deepEqual(GAME_EXPORTS, [
    "start_game",
    "game_command",
    "game_snapshot",
    "game_take_persist",
    "game_server_schema",
    "game_server_document",
  ]);
});

test("server edits are synchronous and surface the engine's refusal", () => {
  const sent = [];
  const wasm = {
    game_snapshot: () => JSON.stringify(SNAPSHOT),
    game_command(text) {
      sent.push(JSON.parse(text));
      if (JSON.parse(text).value === 9999) throw new Error("server.edit userMysekaiStamina.normalStamina: above the master maxStamina");
    },
    game_server_schema: () => '{"schemaVersion":2}',
    game_server_document: () => '{"schemaVersion":2,"document":{}}',
  };
  const controller = createGameController({ wasm, writable: true });
  assert.deepEqual(controller.serverEdit({ path: "a", value: 1 }), { ok: false, reason: "The game is not running" });
  controller.poll();
  assert.deepEqual(controller.serverEdit({ path: "userMysekaiStamina.normalStamina", value: 5 }), { ok: true });
  const refused = controller.serverEdit({ path: "userMysekaiStamina.normalStamina", value: 9999 });
  assert.equal(refused.ok, false);
  assert.match(refused.reason, /maxStamina/);
  assert.deepEqual(controller.serverSchema(), { schemaVersion: 2 });
  assert.equal(controller.serverDocument().schemaVersion, 2);
  assert.equal(sent.length, 2);
});
