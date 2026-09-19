// Parse before locating tools, compiling or creating any output directories.
// An unrecognised output flag must never silently publish to the default pkg/.
export function parseWasmBuildArguments(args) {
  const values = new Map();
  const names = new Set(["profile", "renderer", "out-dir"]);
  for (let i = 0; i < args.length; i++) {
    const match = /^--([^=]+)(?:=(.*))?$/.exec(args[i]);
    if (!match || !names.has(match[1]))
      throw new Error(`Unknown WASM build argument: ${args[i]}`);
    const name = match[1];
    if (values.has(name))
      throw new Error(`Repeated WASM build argument: --${name}`);
    const value = match[2] ?? args[++i];
    if (
      typeof value !== "string" ||
      value.length === 0 ||
      value.startsWith("--")
    ) {
      throw new Error(`--${name} requires a value`);
    }
    values.set(name, value);
  }
  const profile = values.get("profile") ?? "wasm-size";
  if (!["release", "wasm-size", "wasm-names"].includes(profile))
    throw new Error(`Unsupported WASM profile: ${profile}`);
  const renderer = values.get("renderer") ?? "both";
  if (!["webgpu", "webgl2", "both"].includes(renderer))
    throw new Error(`Unsupported renderer: ${renderer}`);
  return {
    profile,
    renderer: renderer === "both" ? null : renderer,
    outDir: values.get("out-dir") ?? null,
  };
}
