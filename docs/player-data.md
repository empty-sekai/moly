# Importing player data

Open **Settings (F10) → Player data**, choose the region that matches the resources, enter a UID and click **Fetch player**. Alternatively, use **Choose JSON** to read a player data file. The native program also accepts a dragged-in JSON file. After confirming the preview, click **Import preview** to load the furniture layout, colour schemes and the area size for the level, and enter the matching scene. On restart, the imported home opens by default. An explicit site parameter still overrides the starting location.

- Accepts raw Home responses and `updatedResources` wrappers. The JSON size limit is 32 MiB.
- Before importing, a backup of the previous layout is kept, and **Restore backup** can restore it. Volume, graphics quality and other settings keep their current values.
- The save is not overwritten when there are unsaved furniture edits, the data is invalid, or the resources do not match. If the limit is exceeded, the import is cancelled, or reading fails, neither the current layout nor the save is changed. A process reads only one import payload at a time, and a result that arrives late does not enter the preview.
- Custom images, decorative pieces, and wallpaper and floor records are kept, but rendering of these appearances is not yet connected.

The resource root must provide `fixture-models/player-data.json` and the colour textures it references. This directory contains the master tables for regions, furniture, areas and level unlocks, but no player data. Each region needs its own master tables and model resources.

**Fetch player** reads the data from the URL template at `player_data.api` (`MOLY_PLAYER_API`), an HTTP(S) URL that names both placeholders, e.g. `https://players.example/api/{region}/mysekai/{target_user_id}`. The program carries no template of its own: without one, **Fetch player** is refused with a message naming `player_data.api` and `MOLY_PLAYER_API`, and the status line shows it. **Choose JSON** and a dragged-in file import a local player-data file without any template.

The browser version uses the same-origin path `/player-api/{region}/{target_user_id}`; the page answers it only from the template its environment gives (`MOLY_PLAYER_API`) and otherwise refuses it. The development server `web/serve.mjs` forwards the path to the template at `MOLY_PLAYER_API` and reports when it is unset; it never forwards to an address of its own. `web/player-api-worker.mjs` does the same for a host with Cloudflare static asset bindings, reading `MOLY_PLAYER_API` from its environment and refusing the path with a message that names it when it is unset.
