// The page side of the game ABI (schemaVersion 1, abi 1): commands into
// `game_command`, the published projection from `game_snapshot`, and the
// persisted documents from `game_take_persist`. Every decision about what a
// command does stays in the engine; this module only validates shapes.
export const GAME_SCHEMA = 1;
export const GAME_ABI = 1;
export const GAME_EXPORTS = Object.freeze([
  "start_game",
  "game_command",
  "game_snapshot",
  "game_take_persist",
]);
const LIFECYCLE = ["hidden", "visible", "pagehide"];
const COVERS = ["opaque", "fading", "gone"];
const MAX_PENDING = 64;
const revisionOf = (value) => Number.isSafeInteger(value) && value >= 0;

/** A validated command in its wire form; unknown commands never pass. */
export function gameCommand(type, value) {
  if (type === "lifecycle" && LIFECYCLE.includes(value)) return { type, value };
  if (type === "input" && typeof value === "boolean")
    return { type, captured: value };
  if (type === "persist.ack" && revisionOf(value))
    return { type, revision: value };
  throw new Error(`Unknown game command: ${String(type)}`);
}

export function isGameSnapshot(value) {
  const entry = value?.entry,
    persist = value?.persist;
  return (
    !!value &&
    typeof value === "object" &&
    value.schemaVersion === GAME_SCHEMA &&
    value.abi === GAME_ABI &&
    !!entry &&
    COVERS.includes(entry.cover) &&
    typeof entry.step === "string" &&
    typeof entry.controlOpen === "boolean" &&
    typeof entry.hudOpen === "boolean" &&
    typeof value.writable === "boolean" &&
    !!persist &&
    revisionOf(persist.revision) &&
    revisionOf(persist.acked) &&
    // null: the engine has not measured its memory (yet).
    (value.memory?.wasmBytes === null || Number.isSafeInteger(value.memory?.wasmBytes)) &&
    Array.isArray(value.errors) &&
    value.errors.every((error) => typeof error === "string")
  );
}

/** `game_take_persist` text: "" when nothing is newer than `after`. */
export function parsePersist(text, after) {
  if (text === "") return null;
  if (typeof text !== "string") throw new Error("Invalid persisted documents");
  const value = JSON.parse(text);
  if (
    !value ||
    typeof value !== "object" ||
    !revisionOf(value.revision) ||
    value.revision <= after ||
    !value.documents ||
    typeof value.documents !== "object" ||
    typeof value.documents.settings !== "string"
  )
    throw new Error("Invalid persisted documents");
  return { revision: value.revision, documents: { settings: value.documents.settings } };
}

/**
 * Created right after `start_game`. `post(type, value)` receives "snapshot"
 * (only when the published text changed) and "error" ({ code, ... }).
 * Commands wait in a bounded queue until the first snapshot has passed the
 * identity check (schema, ABI and the writable lease the seed carried).
 */
export function createGameController({ wasm, post = () => {}, writable }) {
  let closed = false,
    verified = false,
    last = null,
    lastSerialized = "";
  const pending = [];
  const issue = (item) => {
    try {
      wasm.game_command(JSON.stringify(item));
      return true;
    } catch (error) {
      post("error", { code: "command_refused", type: item.type, reason: String(error) });
      return false;
    }
  };
  const close = (code, detail = {}) => {
    closed = true;
    pending.length = 0;
    post("error", { code, ...detail });
  };
  const controller = {
    command(type, value) {
      if (closed) return false;
      const item = gameCommand(type, value);
      if (verified) return issue(item);
      if (pending.length >= MAX_PENDING)
        throw new Error("Game command queue is full");
      pending.push(item);
      return true;
    },
    poll() {
      if (closed) return null;
      const serialized = wasm.game_snapshot();
      if (serialized === lastSerialized) return last;
      let state;
      try {
        state = JSON.parse(serialized);
      } catch {
        state = null;
      }
      if (!isGameSnapshot(state)) {
        close("abi_mismatch", { expected: { schemaVersion: GAME_SCHEMA, abi: GAME_ABI } });
        return null;
      }
      if (!verified) {
        if (state.writable !== writable) {
          close("writable_mismatch", { expected: writable, actual: state.writable });
          return null;
        }
        verified = true;
        for (const item of pending.splice(0)) issue(item);
      }
      last = state;
      lastSerialized = serialized;
      post("snapshot", state);
      return state;
    },
    takePersist(after) {
      if (closed || !verified || !revisionOf(after)) return null;
      return parsePersist(wasm.game_take_persist(after), after);
    },
    ack(revision) {
      return controller.command("persist.ack", revision);
    },
    /** Frames the engine has measured, or null when it publishes no counter. */
    presentedFrames() {
      if (typeof wasm.perf_snapshot !== "function") return null;
      try {
        const perf = JSON.parse(wasm.perf_snapshot(1));
        return perf.available === true && Number.isSafeInteger(perf.frames)
          ? perf.frames
          : null;
      } catch {
        return null;
      }
    },
    getSnapshot() {
      return last;
    },
  };
  return controller;
}
