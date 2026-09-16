// The public host protocol is deliberately separate from the Rust library schema.
// It carries catalogue intents, never entity IDs, script source or gameplay commands.
export const EMBED_VERSION = 2;
export const LIBRARY_VERSION = 1;
export const TABS = Object.freeze([
  "conversations",
  "furniture",
  "performances",
  "activities",
]);
export const LOCALES = Object.freeze([
  "zh-CN",
  "zh-TW",
  "en-US",
  "ja-JP",
  "ko-KR",
]);
export const MAX_PENDING_INTENTS = 64;

export function contentKey(value) {
  if (typeof value !== "string" || value.length > 160)
    throw new TypeError("Invalid content identity");
  const parts = value.split(":");
  const numbers =
    value.startsWith("talk:") &&
    ["general", "fixture"].includes(parts[1]) &&
    parts.length === 3
      ? parts.slice(2)
      : parts[0] === "fixture" && parts.length === 2
        ? parts.slice(1)
        : parts[0] === "activity" &&
            ["notalk", "preaction"].includes(parts[1]) &&
            parts.length === 4
          ? parts.slice(2)
          : [];
  if (
    !numbers.length ||
    numbers.some((x) => !/^[1-9]\d*$/.test(x) || Number(x) > 2147483647)
  )
    throw new TypeError("Invalid content identity");
  return value;
}
function integer(value, min, max, name) {
  if (!Number.isSafeInteger(value) || value < min || value > max)
    throw new TypeError(`Invalid ${name}`);
  return value;
}
function choice(value, values, name) {
  if (!values.includes(value)) throw new TypeError(`Invalid ${name}`);
  return value;
}
export function locale(value = "zh-CN") {
  return choice(value, LOCALES, "locale");
}
export function theme(value = "light") {
  const object = typeof value === "string" ? { mode: value } : value;
  if (!object || typeof object !== "object")
    throw new TypeError("Invalid theme");
  const result = {
    mode: choice(object.mode, ["light", "dark"], "color scheme"),
  };
  if (object.accent !== undefined) {
    if (
      typeof object.accent !== "string" ||
      !/^#[\da-f]{6}$/i.test(object.accent)
    )
      throw new TypeError("Invalid accent");
    result.accent = object.accent;
  }
  return result;
}
export function filters(value = {}) {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new TypeError("Invalid filters");
  const result = {};
  for (const [name, item] of Object.entries(value)) {
    switch (name) {
      case "tab":
        result.tab = choice(item, TABS, name);
        break;
      case "query":
        if (typeof item !== "string") throw new TypeError("Invalid query");
        result.query = Array.from(
          item.replace(/[\u0000-\u001f\u007f]/g, " ").trim(),
        )
          .slice(0, 200)
          .join("");
        break;
      case "character":
        result.character =
          item === null ? null : integer(item, 1, 2147483647, name);
        break;
      case "fixture":
        result.fixture =
          item === null ? null : integer(item, 1, 2147483647, name);
        break;
      case "availability":
        result.availability = choice(item, ["all", "ready", "here"], name);
        break;
      case "mode":
        result.mode = choice(item, ["independent", "current"], name);
        break;
      case "page":
        result.page = integer(item, 0, 1000000, name);
        break;
      case "pageSize":
        result.pageSize = integer(item, 1, 100, name);
        break;
      default:
        throw new TypeError(`Unknown filter: ${name}`);
    }
  }
  return result;
}
export function intent(type, value) {
  switch (type) {
    case "browse":
      return { type, value: filters(value) };
    case "select":
    case "preview":
    case "play":
      return { type, value: contentKey(value) };
    case "stop":
    case "restore":
    case "close":
      return { type };
    default:
      throw new TypeError("Unsupported catalogue intent");
  }
}
export function isEnvelope(data, source) {
  return (
    data &&
    typeof data === "object" &&
    !Array.isArray(data) &&
    data.source === source &&
    data.schemaVersion === EMBED_VERSION &&
    typeof data.type === "string"
  );
}
export function isSnapshot(value) {
  return (
    value &&
    typeof value === "object" &&
    value.schemaVersion === LIBRARY_VERSION &&
    Array.isArray(value.rows) &&
    value.rows.length <= 100 &&
    Array.isArray(value.characters) &&
    value.characters.length <= 1000 &&
    value.status &&
    typeof value.status.canStop === "boolean" &&
    TABS.includes(value.tab)
  );
}
export function sameOriginDirectory(value, base) {
  const url = new URL(value, base);
  if (
    url.origin !== new URL(base).origin ||
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    !url.pathname.endsWith("/") ||
    /%2f|%5c|%00/i.test(url.pathname)
  )
    throw new TypeError("Expected a same-origin directory");
  return url;
}
