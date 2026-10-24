# Master tables and regions

Master tables are read from remote sources by table name. The first source is the regional master table repositories of [Team-Haruki](https://github.com/Team-Haruki) (`haruki-sekai-master`, `haruki-sekai-sc-master`, `haruki-sekai-tc-master`, `haruki-sekai-kr-master`, `haruki-sekai-en-master`), and the mirrors are read after it, in order. `MOLY_MASTER_HARUKI_BASE` replaces the first source, for example `https://raw.githubusercontent.com/Team-Haruki/{repo}/`. `MOLY_MASTER_BASE` and `MOLY_MASTER_MIRROR_BASE` replace the two sources after it.

Every table the game uses is a merged table:

- All rows from the JP region come first, in their original order. They are followed by the rows of each enabled other region whose primary keys do not appear in the JP region. These rows have a `"moly:region"` field that names their source region.
- The primary key defaults to `id`. A few tables use a composite primary key; for example, `resourceBoxes` uses `resourceBoxPurpose` and `id`. Single-object tables use the JP region only.
- If two other regions contribute the same primary key, the whole table is rejected, and the error names the table, the primary key and the regions. If a row has no primary key, or a table cannot be read from another region, that table uses only the JP region's rows, and the reason is reported with the table name.

Currently only the JP region is enabled. Other regions are added once the resource library provides the resources that their exclusive rows reference.
