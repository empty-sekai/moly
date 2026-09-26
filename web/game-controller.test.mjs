import test from "node:test";
import assert from "node:assert/strict";
import { GAME_EXPORTS, gameCommand, isGameSnapshot, parsePersist } from "./game-controller.mjs";

// The wire forms of game ABI 1, as the engine publishes and accepts them.
const SNAPSHOT = {
  schemaVersion: 1,
  abi: 1,
  entry: { cover: "opaque", step: "Title", controlOpen: false, hudOpen: false },
  writable: true,
  persist: { revision: 7, acked: 6 },
  memory: { wasmBytes: 123456789 },
  errors: ["refused"],
};

test("game commands serialize to the ABI 1 wire forms and nothing else", () => {
  for (const value of ["hidden", "visible", "pagehide"])
    assert.deepEqual(gameCommand("lifecycle", value), { type: "lifecycle", value });
  assert.deepEqual(gameCommand("input", true), { type: "input", captured: true });
  assert.deepEqual(gameCommand("persist.ack", 7), { type: "persist.ack", revision: 7 });
  for (const [type, value] of [["lifecycle", "frozen"], ["input", 1], ["persist.ack", -1], ["site", "home_site"]])
    assert.throws(() => gameCommand(type, value), /Unknown game command/);
});

test("the ABI 1 snapshot and persist forms are admitted; other versions are not", () => {
  assert.equal(isGameSnapshot(SNAPSHOT), true);
  for (const cover of ["fading", "gone"])
    assert.equal(isGameSnapshot({ ...SNAPSHOT, entry: { ...SNAPSHOT.entry, cover } }), true);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, abi: 2 }), false);
  assert.equal(isGameSnapshot({ ...SNAPSHOT, schemaVersion: 2 }), false);
  assert.equal(parsePersist("", 7), null);
  assert.deepEqual(parsePersist('{"revision":8,"documents":{"settings":"{}"}}', 7),
    { revision: 8, documents: { settings: "{}" } });
  assert.throws(() => parsePersist('{"revision":7,"documents":{"settings":"{}"}}', 7));
  assert.deepEqual(GAME_EXPORTS, ["start_game", "game_command", "game_snapshot", "game_take_persist"]);
});
