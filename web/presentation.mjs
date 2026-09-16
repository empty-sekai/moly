// Browser-only presentation helpers; runtime remains the authority for eligibility.
export const TABS = [
  "conversations",
  "furniture",
  "performances",
  "activities",
];
export function assetUrl(base, relative) {
  if (
    typeof relative !== "string" ||
    !relative ||
    relative.includes("\\") ||
    /[:?#%\x00-\x20]/.test(relative) ||
    relative.startsWith("/") ||
    relative.split("/").some((p) => !p || p === "." || p === "..")
  )
    return null;
  return base + relative;
}
export function validAssetBase(value) {
  return (
    typeof value === "string" &&
    value.startsWith("/") &&
    !value.startsWith("//") &&
    value.endsWith("/") &&
    !/[\\%?#\x00-\x20]/.test(value) &&
    !value.split("/").some((p) => p === "." || p === "..")
  );
}
export function sourceLabel(region, version) {
  const name =
    { cn: "国服", jp: "日服", tw: "台服", en: "国际服", kr: "韩服" }[region] ??
    "资源快照";
  return version && version !== "unknown" ? `${name} · ${version}` : name;
}
export function safeColor(color) {
  return /^#[0-9a-f]{6}$/i.test(color ?? "") ? color : null;
}
export function initialSelection(url) {
  const p = new URL(url).searchParams;
  return {
    tab: TABS.includes(p.get("tab")) ? p.get("tab") : "conversations",
    query: (p.get("q") ?? "").slice(0, 200),
    key: (p.get("content") ?? "").slice(0, 160),
  };
}
export function initialFilters(url) {
  const p = new URL(url).searchParams;
  const character = Number(p.get("character"));
  const page = Number(p.get("page"));
  const fixture = Number(p.get("fixture"));
  return {
    character:
      Number.isSafeInteger(character) && character > 0 ? character : null,
    availability: ["all", "ready", "here"].includes(p.get("availability"))
      ? p.get("availability")
      : "all",
    mode: p.get("experience") === "current" ? "current" : "independent",
    page: Number.isSafeInteger(page) && page > 0 ? page : 0,
    fixture: Number.isSafeInteger(fixture) && fixture > 0 ? fixture : null,
  };
}
export function pageSummary(total, page, size) {
  return total
    ? `${Math.min(page * size + 1, total)}–${Math.min((page + 1) * size, total)} / ${total.toLocaleString("zh-CN")}`
    : "0 项";
}
export function readableError(error) {
  const reason = String(error?.message ?? error ?? "");
  if (/GPU|adapter|WebGL|WebGPU|render/i.test(reason))
    return "暂时无法启动画面。可以重新加载，或尝试兼容模式。";
  if (/fetch|network|load|download|timeout|超时/i.test(reason))
    return "资源暂时未能加载。请检查连接后重新加载，已有保存内容不会受影响。";
  return "运行遇到了问题，请重新加载后再试。";
}
