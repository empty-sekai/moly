# Master tables and regions

Master tables are read from remote sources by table name. The first source is the regional master table repositories of [Team-Haruki](https://github.com/Team-Haruki) (`haruki-sekai-master`, `haruki-sekai-sc-master`, `haruki-sekai-tc-master`, `haruki-sekai-kr-master`, `haruki-sekai-en-master`), and the mirrors are read after it, in order. `MOLY_MASTER_HARUKI_BASE` replaces the first source, for example `https://raw.githubusercontent.com/Team-Haruki/{repo}/`. `MOLY_MASTER_BASE` and `MOLY_MASTER_MIRROR_BASE` replace the two sources after it.

A resource root that includes the producer's `master` identity in `source.json` pins that region's tables to the recorded commit. The identity must name the region's `Team-Haruki` repository, a full lowercase Git commit, `dataVersion`, `assetVersion` and `assetHash`; the last two must match `source`. The runtime first reads `versions/current_version.json` at that commit and checks all three version fields. The native game, server and browser use this same reader, including deferred tables.

Pinned reads use the existing admitted HTTPS repository base and its `{repo}` template. A missing table, unavailable repository, or mismatched version is reported as a master error; the runtime does not substitute a moving mirror's current tables. A root without master metadata retains the latest repository and mirror behavior. This selects the tables used to interpret a new account's existing defaults; it does not change the account defaults or grant inventory.

Every table the game uses is a merged table:

- All rows from the JP region come first, in their original order. They are followed by the rows of each enabled other region whose primary keys do not appear in the JP region. These rows have a `"moly:region"` field that names their source region.
- The primary key defaults to `id`. A few tables use a composite primary key; for example, `resourceBoxes` uses `resourceBoxPurpose` and `id`. Single-object tables use the JP region only.
- If two other regions contribute the same primary key, the whole table is rejected, and the error names the table, the primary key and the regions. If a row has no primary key, or a table cannot be read from another region, that table uses only the JP region's rows, and the reason is reported with the table name.

Currently only the JP region is enabled. Other regions are added once the resource library provides the resources that their exclusive rows reference.
