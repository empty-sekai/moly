# Source-qualified runtime resources

The browser only loads the selected immutable `/moly/snapshots/<id>/assets/` directory. The host never swaps CN/JP data, accepts an arbitrary fetch URL from a browser, or passes Unity AssetBundles into the GLB loader.

## Local converted runtime

In the existing publication configuration, use `sources: [{ region: "cn", assets: "/private/runtime-cn", catalog: "/private/catalog-cn.json", portraits: "/private/portraits-cn" }]`. `assets` is an already extracted/prepared runtime. Local directory selection is explicit; there is no hidden remote fallback.

## Moesekai asset storage or a configured HTTPS mirror

Upload an already converted runtime and its pinned `runtime-source.json` to a private-operator-selected HTTPS directory. Generate the manifest with `createResourceIndex(root, region, version)` from `resource-sources.mjs`. The result includes `indexSha256`; keep that hash in deployment configuration, not in a URL query.

Use `sources: [{ region: "jp", version: "6.8.1", remote: { kind: "moesekai-asset", baseUrl: "https://<your-asset-host>/<converted-snapshot>/", indexSha256: "<64 lowercase hex characters>" }, catalog: "/private/catalog-jp.json", portraits: "/private/portraits-jp" }]`. Other trusted mirrors use kind `remote`. Do not specify `assets` together with `remote`.

Publication downloads into a hash-qualified private source cache, verifies the manifest pin, every file length/SHA-256 and the actual master region/version, then uses the same existing publication checks as local sources. Four bounded download workers stream to exclusive temporary files; a failed or mismatched file never becomes a successful published asset. Existing verified files are reused. HTTPS only; no redirects, credentials, query strings, traversal, symlinks or wrong-source fallbacks. Ordinary browser pages do not trigger this operator provisioning flow.

`resourceProvider`, `resourceIndexSha256` and `resourceOrigin` are exposed as source provenance. The final server asset volume must remain read-only for an immutable snapshot. Change the source or any portrait, then publish a new snapshot; never replace bytes under an old identity.

This contract does not assert that an existing Moesekai CDN already contains the converted runtime. Raw game resource storage must first pass through the source extraction/preparation pipeline.
