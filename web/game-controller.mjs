// The page side of the game ABI (schemaVersion 1, abi 2): commands into
// `game_command`, the published projection from `game_snapshot`, the
// persisted documents from `game_take_persist`, and the server model's panel
// views from `game_server_schema` / `game_server_document`. Every decision
// about what a command does stays in the engine; this module only validates
// shapes.
export const GAME_SCHEMA = 1;
export const GAME_ABI = 2;
export const GAME_EXPORTS = Object.freeze([
  "start_game",
  "game_command",
  "game_snapshot",
  "game_take_persist",
  "game_server_schema",
  "game_server_document",
]);
/** The page documents, in the order the engine offers them. */
export const GAME_DOCUMENTS = Object.freeze(["settings", "server", "local"]);
const SERVER_ACTIONS = ["gate.reserve", "gate.change", "sync"];
const MAX_SERVER_EDIT = 16 * 1024;
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
  if (type === "server.edit" && isServerEdit(value)) return { type, ...value };
  throw new Error(`Unknown game command: ${String(type)}`);
}

/** `{path, value}` or `{action, ...fields}`; the engine validates the rest. */
function isServerEdit(value) {
  if (!value || typeof value !== "object" || Array.isArray(value) || "type" in value) return false;
  if (JSON.stringify(value).length > MAX_SERVER_EDIT) return false;
  if (typeof value.action === "string")
    return SERVER_ACTIONS.includes(value.action) && !("path" in value);
  return typeof value.path === "string" && value.path !== "" && "value" in value &&
    Object.keys(value).length === 2;
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
    // null until the server model is installed.
    (value.server?.revision === null || revisionOf(value.server?.revision)) &&
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
    Object.keys(value.documents).length !== GAME_DOCUMENTS.length ||
    GAME_DOCUMENTS.some(
      (name) => value.documents[name] !== null && typeof value.documents[name] !== "string",
    )
  )
    throw new Error("Invalid persisted documents");
  const documents = {};
  for (const name of GAME_DOCUMENTS) documents[name] = value.documents[name];
  return { revision: value.revision, documents };
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
    /** The server panel's field description, parsed. */
    serverSchema() {
      if (closed || !verified) return null;
      return JSON.parse(wasm.game_server_schema());
    },
    /** The server document with its clock, client copies and refusals. */
    serverDocument() {
      if (closed || !verified) return null;
      return JSON.parse(wasm.game_server_document());
    },
    /**
     * One `server.edit`, applied synchronously. Resolves to `{ ok: true }` or
     * `{ ok: false, reason }` with the engine's named refusal.
     */
    serverEdit(edit) {
      if (closed || !verified) return { ok: false, reason: "The game is not running" };
      const item = gameCommand("server.edit", edit);
      try {
        wasm.game_command(JSON.stringify(item));
        return { ok: true };
      } catch (error) {
        return { ok: false, reason: String(error) };
      }
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
