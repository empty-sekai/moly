// Hosts publish an explicit list of complete, separately mounted snapshots.
import { validAssetBase, sourceLabel } from "./presentation.mjs";

export function snapshotUrl(currentUrl, assetBase) {
  if (!validAssetBase(assetBase)) throw new Error("Invalid resource snapshot");
  const url = new URL(currentUrl);
  url.searchParams.set("assets", assetBase);
  // IDs, labels, searches and player previews belong to one resource snapshot.
  for (const key of [
    "content",
    "fixture",
    "character",
    "page",
    "q",
    "packs",
    "player_uid",
  ])
    url.searchParams.delete(key);
  return url.href;
}

export async function installSnapshotPicker(assetBase) {
  const anchor = document.getElementById("source-label");
  if (!anchor || document.getElementById("snapshot-picker")) return;
  try {
    const response = await fetch(new URL("./snapshots.json", location.href));
    if (!response.ok) return;
    const document = await response.json();
    const snapshots = (
      Array.isArray(document.snapshots) ? document.snapshots : []
    ).filter((row) => row && validAssetBase(row.assetBase));
    if (
      snapshots.length < 2 ||
      !snapshots.some((row) => row.assetBase === assetBase)
    )
      return;
    const picker = globalThis.document.createElement("select");
    picker.id = "snapshot-picker";
    picker.className = "source-chip";
    picker.setAttribute("aria-label", "选择资源区服");
    for (const row of snapshots)
      picker.add(
        new Option(sourceLabel(row.region, row.version), row.assetBase),
      );
    picker.value = assetBase;
    picker.addEventListener("change", () => {
      location.assign(snapshotUrl(location.href, picker.value));
    });
    anchor.replaceWith(picker);
    // Keep the status label available to the shell without duplicating chrome.
    anchor.hidden = true;
    picker.after(anchor);
  } catch {
    /* The host's snapshot menu is optional. */
  }
}
