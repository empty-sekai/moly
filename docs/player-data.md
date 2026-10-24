# Importing player data

Open **Settings (F10) → Player data**, choose the region that matches the resources, enter a UID and click **Fetch player**. Alternatively, use **Choose JSON** to read a player data file. The native program also accepts a dragged-in JSON file. After confirming the preview, click **Import preview** to load the furniture layout, colour schemes and the area size for the level, and enter the matching scene. On restart, the imported home opens by default. An explicit site parameter still overrides the starting location.

- Accepts raw Home responses and `updatedResources` wrappers. The JSON size limit is 32 MiB.
- Before importing, a backup of the previous layout is kept, and **Restore backup** can restore it. Volume, graphics quality and other settings keep their current values.
- The save is not overwritten when there are unsaved furniture edits, the data is invalid, or the resources do not match. If the limit is exceeded, the import is cancelled, or reading fails, neither the current layout nor the save is changed. A process reads only one import payload at a time, and a result that arrives late does not enter the preview.
- Custom images, decorative pieces, and wallpaper and floor records are kept, but rendering of these appearances is not yet connected.

The resource root must provide `fixture-models/player-data.json` and the colour textures it references. This directory contains the master tables for regions, furniture, areas and level unlocks, but no player data. Each region needs its own master tables and model resources.

By default, the native program fetches data from `https://haruki-api.menardi.top/api/{region}/mysekai/{target_user_id}`. `MOLY_PLAYER_API` replaces this template. The browser version uses the same-origin path `/player-api/{region}/{target_user_id}`. The development server `web/serve.mjs` already includes the forwarding, and `web/player-api-worker.mjs` can be used when Cloudflare static asset bindings are used.
