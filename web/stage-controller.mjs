import {
  LIBRARY_VERSION,
  contentKey,
  filters,
  intent,
  isSnapshot,
} from "./embed-contract.mjs";

const DEFAULTS = Object.freeze({
  tab: "conversations",
  query: "",
  character: null,
  fixture: null,
  availability: "all",
  mode: "independent",
  page: 0,
  pageSize: 24,
});
export function tabForKey(key) {
  return key?.startsWith("activity:")
    ? "activities"
    : key?.startsWith("fixture:")
      ? "furniture"
      : "conversations";
}

// This controller translates presentation intents only. Every admission,
// transition, replacement, completion and restoration decision stays in Rust.
export function createStageController({
  wasm,
  post,
  region,
  version,
  initial = {},
  content = null,
}) {
  let desired = { ...DEFAULTS, ...filters(initial) };
  if (content && !initial.tab) desired.tab = tabForKey(content);
  let pendingSelection = content ? contentKey(content) : null;
  let configured = false,
    applied = false,
    verified = false,
    closed = false,
    closing = false;
  let closeRevision = null,
    last = null,
    lastSerialized = "",
    selectionSent = false;
  const pending = [];
  const command = (type, value = undefined, key = undefined) => {
    wasm.library_command(
      JSON.stringify({
        schemaVersion: LIBRARY_VERSION,
        type,
        ...(value !== undefined ? { value } : {}),
        ...(key ? { key } : {}),
      }),
    );
  };
  const applyFilters = () => {
    command("tab", desired.tab);
    command("fixture", desired.fixture);
    command("mode", desired.mode);
    command("character", desired.character);
    command("query", desired.query);
    command("availability", desired.availability);
    command("pageSize", desired.pageSize);
    command("page", desired.page);
  };
  const issue = (item) => {
    if (item.type === "browse") {
      desired = { ...desired, ...item.value };
      applyFilters();
    } else if (item.type === "select") command("select", undefined, item.value);
    else if (item.type === "play" || item.type === "preview")
      command(item.type, undefined, item.value);
    else if (item.type === "close") {
      closing = true;
      pending.length = 0;
      pendingSelection = null;
      closeRevision = last?.revision;
      command("close");
    } else command(item.type);
  };
  return {
    configure(value = {}) {
      if (configured) return; // theme/locale updates may never rewind the catalogue
      desired = { ...desired, ...filters(value.initial ?? {}) };
      if (value.content) {
        pendingSelection = contentKey(value.content);
        if (!value.initial?.tab) desired.tab = tabForKey(pendingSelection);
      }
      configured = true;
      command("open");
      command("focus", false);
    },
    dispatch(type, value) {
      if (closed || closing) return;
      const item = intent(type, value);
      if (type === "close") {
        issue(item);
        return;
      }
      if (!verified || !applied) {
        if (pending.length >= 64)
          throw new Error("Catalogue intent queue is full");
        pending.push(item);
      } else issue(item);
    },
    focus(captured) {
      if (!closed) command("focus", !!captured);
    },
    getSnapshot() {
      return last;
    },
    poll() {
      if (closed) return null;
      const serialized = wasm.library_snapshot();
      const state = JSON.parse(serialized);
      if (!isSnapshot(state)) return null;
      last = state;
      if (state.ready && !verified) {
        if (state.region !== region || (version && state.version !== version)) {
          closed = true;
          post("error", {
            code: "source_mismatch",
            expected: { region, version },
            actual: { region: state.region, version: state.version },
          });
          return null;
        }
        verified = true;
      }
      if (configured && verified && !applied && !closing) {
        applyFilters();
        applied = true;
        return state;
      }
      if (applied && state.ready && state.tab === desired.tab && !closing) {
        if (pendingSelection) {
          if (!selectionSent) {
            command("select", undefined, pendingSelection);
            selectionSent = true;
            return state;
          }
          if (state.selected?.key !== pendingSelection) {
            post("error", {
              code: "selection_unavailable",
              key: pendingSelection,
            });
          }
          pendingSelection = null;
        }
        for (const item of pending.splice(0)) issue(item);
      }
      if (serialized !== lastSerialized) {
        lastSerialized = serialized;
        post("snapshot", state);
      }
      if (
        closing &&
        state.revision !== closeRevision &&
        !state.status.canStop
      ) {
        closed = true;
        post("closed", { restored: true });
      }
      return state;
    },
  };
}
