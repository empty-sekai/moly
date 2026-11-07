# Master tables and regions

Master tables are read from remote hosts by table name, and every host comes from the configuration file `moly.toml` (or the environment variable that overrides its key). A host that no key names is not read, and a feature that needs one refuses by naming the key. See [configuration](configuration.md) for the file itself.

The browser reads the same keys from its page parameters, which have the same names: `?storage_base=` and `?storage_mirror_base=` for the storage, and `?master_base=`, `?master_mirror_base=`, `?master_repository=`, `?master_repository_jp=` … `?master_repository_en=` for the masters.

## The sources

Master tables are read from three sources, tried in this order:

1. **The repository host**, `masters.repository` (`MOLY_MASTER_REPOSITORY`): an address template with a `{repo}` path segment, for example `https://masters.example/{repo}/`. It carries one repository per region, and `masters.repository_jp`, `masters.repository_cn`, `masters.repository_tw`, `masters.repository_kr` and `masters.repository_en` (`MOLY_MASTER_REPOSITORY_JP` … `MOLY_MASTER_REPOSITORY_EN`) name the repository of each region: one path segment of letters, digits, `.`, `_` and `-`. A table of region `jp` is read from `jp/main/master/<table>.json` with `{repo}` replaced by the JP repository name, and its version document from `jp/main/versions/current_version.json`.
2. **The master host**, `masters.base` (`MOLY_MASTER_BASE`): a directory per region, `<region>/master/<table>.json` and `<region>/versions/current_version.json`.
3. **The master mirror**, `masters.mirror` (`MOLY_MASTER_MIRROR_BASE`): the same layout as the master host, tried after it.

Every base is a canonical HTTPS directory ending in `/`; a template must name `{repo}` exactly once, as a whole path segment. The native program also accepts a `file://` directory ending in `/` (for example `file:///D:/masters/{repo}/`), read in the same layout from disk; the part before `{repo}` must exist. A configured base that is none of these is refused when the program starts, naming its key.

A source whose key is not set is skipped: the sources that are configured are tried in the order above, and a table is only absent once each of them has failed. When no master source is configured at all, the master read fails once with a message naming `masters.repository`, `masters.base` and `masters.mirror`. The upstream data version (`versions/current_version.json`) of the asset root's region is logged once per start.

## Pinning

A resource root that includes the producer's `master` record in `source.json` pins that region's tables to the recorded commit:

```json
"master": {
  "region": "jp",
  "repository": "example-owner/example-jp",
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "dataVersion": "7.0.0.32",
  "assetVersion": "7.0.0.30",
  "assetHash": "assets"
}
```

The record must name the region of `source.region`, a `repository` of the form `owner/name` with both parts non-empty, a full lowercase 40-digit Git commit, `dataVersion`, `assetVersion` and `assetHash`; the last two must match `source`. The repository's `name` must be the configured repository name of that region (`masters.repository_jp` for a `jp` root, and so on); the owner is the record's own. A pinned region whose record names another repository, or that has no configured repository name, is refused by naming the key; it is never read from another repository. The record is checked before any table is read.

A pinned region's tables and version document are read from the repository host under the commit, `jp/<commit>/master/<table>.json` and `jp/<commit>/versions/current_version.json`. The runtime first reads the version document at that commit and checks all three version fields. A missing table, unavailable repository or mismatched version is reported as a master error, and the moving master hosts are not used for that region. A root without a `master` record reads the latest tables from the configured sources in the order above. The native game, server and browser use this same reader, including deferred tables.

## Merging

Every table the game uses is a merged table:

- All rows from the JP region come first, in their original order. They are followed by the rows of each enabled other region whose primary keys do not appear in the JP region. These rows have a `"moly:region"` field that names their source region.
- The primary key defaults to `id`. A few tables use a composite primary key; for example, `resourceBoxes` uses `resourceBoxPurpose` and `id`. Single-object tables use the JP region only.
- If two other regions contribute the same primary key, the whole table is rejected, and the error names the table, the primary key and the regions. If a row has no primary key, or a table cannot be read from another region, that table uses only the JP region's rows, and the reason is reported with the table name.

Currently only the JP region is enabled. Other regions are added once the resource library provides the resources that their exclusive rows reference.
