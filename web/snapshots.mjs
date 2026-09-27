// Hosts publish an explicit list of complete, separately mounted snapshots.
import { applyPackSelection, validCatalogId } from "./asset-pack-client.mjs";
import { validAssetBase, sourceLabel } from "./presentation.mjs";

export function snapshotUrl(currentUrl, assetBase, selection = {}) {
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
  applyPackSelection(url, selection);
  return url.href;
}

export async function installSnapshotPicker(assetBase) {
  const anchor = document.getElementById("source-label");
  if (!anchor || document.getElementById("snapshot-picker")) return;
  try {
    const response = await fetch(new URL("./snapshots.json", location.href), {
      cache: "no-store",
    });
    if (!response.ok) return;
    const document = await response.json();
    const snapshots = (
      Array.isArray(document.snapshots) ? document.snapshots : []
    ).filter(
      (row) =>
        row &&
        validAssetBase(row.assetBase) &&
        (row.assetCatalog === undefined || validCatalogId(row.assetCatalog)),
    );
    if (
      snapshots.length < 2 ||
      !snapshots.some((row) => row.assetBase === assetBase)
    )
      return;
    const picker = globalThis.document.createElement("select");
    picker.id = "snapshot-picker";
    picker.className = "source-chip";
    picker.setAttribute("aria-label", "选择资源区服");
    const selectedCatalog = new URL(location.href).searchParams.get(
      "asset_catalog",
    );
    const key = (row) =>
      row.assetCatalog ? `${row.assetBase}#${row.assetCatalog}` : row.assetBase;
    snapshots.forEach((row) =>
      picker.add(new Option(sourceLabel(row.region, row.version), key(row))),
    );
    const selected = snapshots.find(
      (row) =>
        row.assetBase === assetBase &&
        (row.assetCatalog ?? null) === selectedCatalog,
    );
    // A pinned historical release may be absent from current discovery. Keep
    // its actual source label instead of silently displaying another region.
    if (!selected) return;
    picker.value = key(selected);
    picker.addEventListener("change", () => {
      const row = snapshots.find(
        (candidate) => key(candidate) === picker.value,
      );
      if (row) location.assign(snapshotUrl(location.href, row.assetBase, row));
    });
    anchor.replaceWith(picker);
    // Keep the status label available to the shell without duplicating chrome.
    anchor.hidden = true;
    picker.after(anchor);
  } catch {
    /* The host's snapshot menu is optional. */
  }
}
