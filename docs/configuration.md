# Configuration

## The configuration file

Settings are kept in one TOML file, `moly.toml`. The file is `moly.toml` in the [data directory](../crates/app/moly-launcher/README.md); the environment variable `MOLY_CONFIG` names another file when it is set and not empty (see [Before the file](#before-the-file)). A missing file is an empty configuration: every key takes its default, and a key without a default is refused by name when a program needs it.

The launcher, the game, the developer window and the server read the same file. For every key, a value comes from its environment variable when that variable is set, else from the file, else the default of the program that reads it. An environment variable set to an empty value counts as set for text, path and address keys (the program that needs the key refuses it by name), and as unset for flags.

Only a few keys have a default. The server address given to the native game defaults to the local server `http://127.0.0.1:15800`, and the update source to the program's own release source. The addresses of the resources, the master tables, the player data and the server's own listen address have no default: a feature that needs one stops with a message naming the key, to be set in the file or through its environment variable.

The file is checked when it is loaded. A key that is not declared, a value outside a section, or a value of the wrong kind (for example `hide_hud = "yes"`, or an address that does not start with `http://` or `https://`) stops the start with the key named.

[moly.example.toml](../moly.example.toml) lists every key as a comment: copy the lines you need into `moly.toml` and remove the `# `.

The launcher edits `moly.toml` in place when its settings change, and writes its keys (`resources.base`, `resources.dir`, `resources.snapshot`, `launcher.jobs`, `launcher.game`, `update.source` and `dev.window`). It keeps the comments and the order of the file and changes only the value of each key it sets; a key it no longer sets is removed. `moly.toml` is created when it does not exist. On its first start, a launcher that finds an older `launcher.json` in the data directory, and none of its own keys in `moly.toml`, imports the file's values into `moly.toml` and renames it `launcher.json.imported`.

The settings, server documents and local list are not stored in this file. They are kept separately: in the settings file (`paths.settings`, by default `settings.json` in the data directory), and in `settings.server.json` and `settings.local.json` next to it. The server's saves are its player documents (`server.data`). All of this is runtime state, not configuration: a program writes it while it runs, and a second process that runs at the same time only reads it.

## Keys

Each key is written `name` in a `[section]` of `moly.toml`, so its full path is `section.name`. "Read by" names the programs that read the key.

### `[paths]`

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `settings` | `MOLY_SETTINGS_FILE` | path | game, developer window | The settings file. Default: `settings.json` in the data directory. |

### `[resources]`

The game reads its resources from the resource root, from the asset store, or from the resources the launcher installed, in that order; see [Building from source](building.md). The asset store keys apply to the game only, while the remote files it reads come from the storage of `storage`, with `storage_mirror` tried next. `root` and `store` cannot both be set, and `catalog` and `cache` apply to whichever of them is read.

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `root` | `MOLY_ASSET_ROOT` | path | game, developer window, stage | The resource root directory. Default: the version selected in the launcher, else the newest installed one. |
| `store` | `MOLY_ASSET_STORE_URL` | address | game | A public asset store directory read over HTTPS instead of a local resource root. Setting both is refused. |
| `catalog` | `MOLY_ASSET_CATALOG` | string | game | The catalog release (a lowercase SHA-256) read from the resource root or the asset store. |
| `cache` | `MOLY_ASSET_CACHE` | path | game | Where documents and files read from `resources.store` are kept. Created when missing. |
| `base` | none | address | launcher | The address resources are downloaded from, ending with `/`. Required: a feature that downloads resources stops, naming the key, when it is unset. |
| `dir` | none | path | launcher, game | Where the launcher installs resources. Default: the `resources` directory beside the data directory. |
| `snapshot` | none | string | launcher, game | The installed resource version the game starts with. Default: the newest complete install. |
| `storage` | `MOLY_STORAGE_BASE` | address | game, developer window | The address of the storage that remote resource files are read from. |
| `storage_mirror` | `MOLY_STORAGE_MIRROR_BASE` | address | game, developer window | The storage tried when `resources.storage` does not answer. |

### `[masters]`

Master tables are read from three sources, tried in this order: the repository host `masters.repository`, the master host `masters.base`, and the master mirror `masters.mirror`. The `repository_*` keys name the repository of each region in the repository host. A source whose key is unset is skipped. See [master tables and regions](masters.md).

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `repository` | `MOLY_MASTER_REPOSITORY` | address | game, developer window | The first master table source: an address template with a `{repo}` path segment, which is replaced by the region's repository name. Required by the master read when no other master source is configured; see [master tables and regions](masters.md). |
| `repository_jp` | `MOLY_MASTER_REPOSITORY_JP` | string | game, developer window | The repository name of the `jp` region in `masters.repository`. |
| `repository_cn` | `MOLY_MASTER_REPOSITORY_CN` | string | game, developer window | The repository name of the `cn` region in `masters.repository`. |
| `repository_tw` | `MOLY_MASTER_REPOSITORY_TW` | string | game, developer window | The repository name of the `tw` region in `masters.repository`. |
| `repository_kr` | `MOLY_MASTER_REPOSITORY_KR` | string | game, developer window | The repository name of the `kr` region in `masters.repository`. |
| `repository_en` | `MOLY_MASTER_REPOSITORY_EN` | string | game, developer window | The repository name of the `en` region in `masters.repository`. |
| `base` | `MOLY_MASTER_BASE` | address | game, developer window | The second master table source. Required by the master read when no other master source is configured; see [master tables and regions](masters.md). |
| `mirror` | `MOLY_MASTER_MIRROR_BASE` | address | game, developer window | The third master table source. Required by the master read when no other master source is configured; see [master tables and regions](masters.md). |

### `[player_data]`

See [importing player data](player-data.md). Setting `player_data.uid` (`MOLY_PLAYER_UID`) together with `player_data.file` (`MOLY_PLAYER_DATA_FILE`) stops the start and names both keys.

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `api` | `MOLY_PLAYER_API` | address | developer window | The player data address template, with `{region}` and `{target_user_id}` placeholders. Required: the developer window stops, naming the key, when it imports player data and the template is unset. |
| `region` | `MOLY_PLAYER_REGION` | string | developer window | The region of the player data: `cn` or `jp`. |
| `uid` | `MOLY_PLAYER_UID` | string | developer window | Fetches and previews the data of this user id at start. |
| `file` | `MOLY_PLAYER_DATA_FILE` | path | developer window | Imports this player data file at start. Set at most one of `uid` and `file`. |

### `[server]`

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `url` | `MOLY_SERVER_URL` | address | game, developer window | The server the game connects to. Default: `http://127.0.0.1:15800`, the local server. |
| `listen` | `MOLY_SERVER_ADDR` | string | server | The server's listen address (`host:port`). Default: `127.0.0.1:15800`. |
| `control_port` | `MOLY_SERVER_CONTROL_PORT` | whole number | server | The server's control port. Default: `15801`, always bound to `127.0.0.1`. |
| `data` | `MOLY_SERVER_DATA` | path | server | The directory the server saves player documents in. Unset: documents are kept in memory only. |

The server also reads `MOLY_SERVER_ADMIN`, from the environment only: `off` refuses the administration calls, `on` or unset allows them, and any other value stops the start. A server behind a reverse proxy listens on a loopback address and sets it to `off`.

### `[update]`

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `source` | none | address | launcher | The release source the launcher checks for program updates. Default: the program's own release source. |

### `[launcher]`

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `jobs` | none | whole number | launcher | Parallel transfers while downloading. Default: 16. |
| `game` | none | path | launcher | The game program. Default: the one beside the launcher. |

### `[display]`

`hide_hud` and `hide_player` are `true` or `false` in the file and `1` or `0` in the environment; any other value stops the start with the key named. A plugin or a director script changes the same switches with the `view.set` step (see [Plugins](plugins.md)).

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `hide_hud` | `MOLY_HIDE_HUD` | true or false | game, developer window, stage | Hides every HUD, button and dialog view; the scene, the balloons and the talk window remain. Default: false, the game's own interface. |
| `hide_player` | `MOLY_HIDE_PLAYER` | true or false | game, developer window, stage | Hides the player's own body. Default: false, the body shows. |

### `[text]`

The developer window translates its display text with these files: the source catalogue named by `text.source`, over which the official and community overlays are applied, in the locale of `text.locale`. Without `text.source` the original text is shown.

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `source` | `MOLY_TEXT_SOURCE` | path | developer window | The source text catalogue file the display translation reads. Unset: the original text is shown. |
| `official` | `MOLY_TEXT_OFFICIAL` | path | developer window | An official translation overlay file applied over `text.source`. |
| `community` | `MOLY_TEXT_COMMUNITY` | path | developer window | A community translation overlay file applied over `text.source`. |
| `locale` | `MOLY_TEXT_LOCALE` | string | developer window | The locale of the display translation. Default: `zh-CN`. |

### `[dev]`

These are developer and diagnostic switches, read by the developer window, or by the server where the Read by column says so. Each one is also set through its environment variable. Every `[dev]` key except `dev.window` and `dev.check_systems` holds a string that the switch parses itself, so write it quoted (`seed = "42"`, not `seed = 42`); what the string must look like, and what an invalid one does, is in its Description.

| Key | Environment variable | Value | Read by | Description |
|---|---|---|---|---|
| `window` | `MOLY_DEV` | true or false | game, server | Starts the developer window instead of the game. Default: false. |
| `check_systems` | `MOLY_CHECK_SYSTEMS` | true or false | game | Reports every system parameter conflict and exits instead of starting the game. Default: false. |
| `portraits_out` | `MOLY_PORTRAITS_OUT` | path | developer window | Exports character portraits into this directory, with its own settings file there, instead of starting normally. |
| `portrait_unit` | `MOLY_PORTRAIT_UNIT` | string | developer window | Limits the portrait export to this source unit id. |
| `action_button_autowalk_secs` | `MOLY_ACTION_BUTTON_AUTOWALK_SECS` | string | developer window | A window (`<until>` or `<from>:<until>` seconds) in which the player is walked to each character and fixture and the action button is tapped; unset: the autowalk is off. |
| `audio_mock_music_record` | `MOLY_AUDIO_MOCK_MUSIC_RECORD` | string | server | Sets the default document's `userMysekaiMusicPlayFixtureSettings` from `site:record:vocal` entries separated by commas; a malformed entry stops the start with a panic. Unset: the document's own settings. |
| `avatar_mock_accessory` | `MOLY_AVATAR_MOCK_ACCESSORY` | string | server | The accessory bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own accessory. |
| `avatar_mock_coordinate` | `MOLY_AVATAR_MOCK_COORDINATE` | string | server | The coordinate bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own coordinate. |
| `avatar_mock_costume` | `MOLY_AVATAR_MOCK_COSTUME` | string | server | The costume bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own costume. |
| `avatar_mock_skin_color` | `MOLY_AVATAR_MOCK_SKIN_COLOR` | string | server | The skin colour code the developer avatar wears, turned into its master id once the masters resolve; a code no master row holds is logged. Unset: the document's own skin colour. |
| `balloon_order_no_layer` | `MOLY_BALLOON_ORDER_NO_LAYER` | string | developer window | Feeds 0 as every balloon's root z, so balloons keep the constant layer instead of being ranked by distance from the camera; the negative arm of the layer-order check. Unset: balloons are ranked by distance. |
| `balloon_order_run_id` | `MOLY_BALLOON_ORDER_RUN_ID` | string | developer window | A run id written into the layer-order probe's first log line together with the version. Unset: the line is not written. |
| `balloon_order_secs` | `MOLY_BALLOON_ORDER_SECS` | string | developer window | Spawns a pair of overlapping probe balloons that leave after this many seconds to check balloon layer order (headless acceptance). Read by moly_world::balloon; unset or 0: no probes. |
| `bgm_select_taps` | `MOLY_BGM_SELECT_TAPS` | string | developer window | Comma-separated `<seconds>=<target>` taps on the BGM select screen (`x:y`, `back`, `status`, `vocal`, `cell:<n>`); unset: no taps. |
| `birthday_close_secs` | `MOLY_BIRTHDAY_CLOSE_SECS` | string | developer window | Seconds after the party's close confirm shows before its OK is tapped; unset: no tap. |
| `birthday_confirm_secs` | `MOLY_BIRTHDAY_CONFIRM_SECS` | string | developer window | Seconds after the birthday confirm shows before its OK is tapped; unset: no tap. |
| `birthday_end_secs` | `MOLY_BIRTHDAY_END_SECS` | string | developer window | Seconds after start before the housing screen's end-party button is tapped once its UI shows it; unset: no tap. |
| `birthday_now_ms` | `MOLY_BIRTHDAY_NOW_MS` | string | server | Fixes the server clock at this epoch millisecond, a positive integer, on the default document; any other value stops the start with a panic. Unset: the clock runs from the real time. |
| `birthday_party_action` | `MOLY_BIRTHDAY_PARTY_ACTION` | string | developer window | `<seconds>[:<fixture>]`: places the player beside a party fixture that many seconds after start and taps its birthday cut-scene button; unset: no press. |
| `camera_autofps_secs` | `MOLY_CAMERA_AUTOFPS_SECS` | string | developer window | The FPS smoke hook's window in seconds: synthetic pinch and drag input is fed through the real camera dispatch (pinch in to FPS, drag inside FPS, pinch out) so the transitions can be read from the log. Unset or 0: no synthetic input. |
| `camera_autogesture_secs` | `MOLY_CAMERA_AUTOGESTURE_SECS` | string | developer window | The gesture smoke script's window in seconds: in the Normal camera state four phases drive pitch and distance to their limits through the real camera rules; the script stays off while the FPS hook is on. Unset or 0: no script. |
| `camera_autoreset` | `MOLY_CAMERA_AUTORESET` | string | developer window | Instrument `<state>:<seconds>`: once per entry into that camera state, the camera reset runs after `seconds` as the reset button runs it. A value without both parts is ignored. Unset: no automatic reset. |
| `control_port` | `MOLY_CONTROL_PORT` | string | developer window | Opens the control surface's HTTP endpoint on that port at 127.0.0.1; unset: no HTTP endpoint; anything but a port number stops the start. |
| `convert_mock_request` | `MOLY_CONVERT_MOCK_REQUEST` | string | server | Answers a native convert request: `start:slot:item:count[:choice]`, `receive:slot:count` or `shorten:slot:price`; a malformed value stops the start with a panic. Unset: no convert request is made. |
| `craft_apply_secs` | `MOLY_CRAFT_APPLY_SECS` | string | developer window | Seconds after the craft screen opens before the apply button is pressed; unset: no press. |
| `craft_confirm_secs` | `MOLY_CRAFT_CONFIRM_SECS` | string | developer window | Seconds after the craft confirm shows before its OK is tapped; unset: no tap. |
| `craft_mock_materials` | `MOLY_CRAFT_MOCK_MATERIALS` | string | server | Sets `userMysekaiMaterials` rows from comma-separated `id:quantity` entries (an id of at least 1, a quantity of at least 0); a malformed entry stops the start with a panic. Unset: the document's own materials. |
| `craft_mock_request` | `MOLY_CRAFT_MOCK_REQUEST` | string | server | Makes the home action's craft request a craft of `blueprintId[:quantity[:textureId[:blueprintTermId]]]`; a malformed value stops the start with a panic. Unset: the home action's request is answered as sent. |
| `craft_result_close_secs` | `MOLY_CRAFT_RESULT_CLOSE_SECS` | string | developer window | Seconds after the craft result dialog shows before a tap on its close area; unset: no tap. |
| `craft_select` | `MOLY_CRAFT_SELECT` | string | developer window | `blueprint[xcount]`: the craft selection made when the craft screen opens; unset: no selection. |
| `craft_stage_result` | `MOLY_CRAFT_STAGE_RESULT` | string | developer window | `<count>[,first]`: stages the result dialogs for the selected blueprint without a craft request; unset: no staging. |
| `craft_tap_apply` | `MOLY_CRAFT_TAP_APPLY` | string | developer window | Taps the craft screen's apply button three seconds after it opens when set; unset: no tap. |
| `craft_tap_cell` | `MOLY_CRAFT_TAP_CELL` | string | developer window | The recycled cell slot tapped two seconds after the craft screen opens; unset: none. |
| `craft_tap_tab` | `MOLY_CRAFT_TAP_TAB` | string | developer window | The tab cell index tapped at its drawn centre one second after the craft screen opens; unset: none. |
| `delivery_autoplay` | `MOLY_DELIVERY_AUTOPLAY` | string | developer window | Instrument: `hold[,close]` in seconds walks the player to the delivery drops and into the place circle, holds the delivery button `hold` seconds and closes each awaited dialog after `close` seconds (default 1.5). Unset: no autoplay. |
| `director_script` | `MOLY_DIRECTOR_SCRIPT` | path | developer window | Queues this director script file when a native run starts; unset: no script is queued. |
| `door_walk_after` | `MOLY_DOOR_WALK_AFTER` | string | developer window | The seconds of app time the door walk waits before it starts; unset: it starts at once. |
| `door_walk_offset` | `MOLY_DOOR_WALK_OFFSET` | string | developer window | `x,z` metres the door walk's goal is shifted by; unset: the goal is the door itself. |
| `door_walk_secs` | `MOLY_DOOR_WALK_SECS` | string | developer window | Walks the player to the site's door until that many seconds of app time and taps the door's button; unset or zero: no door walk. |
| `edit_autoplay` | `MOLY_EDIT_AUTOPLAY` | string | developer window | The seconds after the joystick opens before the edit autoplay sends one layout edit (an empty value means 3); unset: no autoplay. |
| `edit_autoplay_catalog` | `MOLY_EDIT_AUTOPLAY_CATALOG` | string | developer window | Comma-separated catalog indices the edit autoplay puts (a trailing `d` also decides the put); unset: `2,0`. |
| `edit_autoplay_clean_up` | `MOLY_EDIT_AUTOPLAY_CLEAN_UP` | string | developer window | Presses the remove-all button and its confirmation before saving; unset: nothing is removed. |
| `edit_autoplay_clean_up_hold` | `MOLY_EDIT_AUTOPLAY_CLEAN_UP_HOLD` | string | developer window | Seconds the edit autoplay waits with the remove-all confirmation open before its clean-up button; unset: no hold. |
| `edit_autoplay_hold` | `MOLY_EDIT_AUTOPLAY_HOLD` | string | developer window | Seconds the edit autoplay idles after the camera buttons with nothing selected, so an outside drag reaches the edit camera; unset: no hold. |
| `edit_autoplay_return_base` | `MOLY_EDIT_AUTOPLAY_RETURN_BASE` | string | developer window | Picks the stack run's base fixture once more and sends it to storage before saving; unset: it is left in place. |
| `edit_autoplay_stack` | `MOLY_EDIT_AUTOPLAY_STACK` | string | developer window | `<uid>,<uid>`: the fixture the stack run picks first and the fixture it is dragged onto; unset: the usual pick. |
| `edit_autoplay_wait_tweet` | `MOLY_EDIT_AUTOPLAY_WAIT_TWEET` | string | developer window | The edit autoplay waits (up to 120 s) for a visible tweet balloon before the edit button; unset: it enters at once. |
| `effect_sample` | `MOLY_EFFECT_SAMPLE` | string | developer window | Also samples each system's live put-effect particles every 0.25 s after a put; unset: no samples. |
| `effect_trace` | `MOLY_EFFECT_TRACE` | string | developer window | Traces each effect instance's draws every frame; unset: no trace. |
| `emoticon_items` | `MOLY_EMOTICON_ITEMS` | string | developer window | A showcase carousel of emotes: comma-separated item names from the archive, refused with a panic when a name is not in the archive or the list is empty. Unset: no carousel. |
| `emoticon_show_secs` | `MOLY_EMOTICON_SHOW_SECS` | string | developer window | The seconds each showcase emote is shown. Unset or not a number: 6. |
| `fixture_clock_now` | `MOLY_FIXTURE_CLOCK_NOW` | string | developer window | Pins the fixture clock's local time to `HH:MM` on native builds so clock gimmicks can be verified at a known hour; a malformed value is refused with a panic. Unset: the device's local clock. |
| `fixture_direction` | `MOLY_FIXTURE_DIRECTION` | string | developer window | Smoke knob that rewrites the facing of every placed fixture to 0=Front, 1=Left, 2=Back or 3=Right; a value outside 0..3 or not an integer is refused with a panic. Unset or empty: the table's own Front facing is kept. |
| `fixture_preview` | `MOLY_FIXTURE_PREVIEW` | string | developer window | The developer model preview to add to the placement table, `surface-<n>` or `gimmick-<n>` in developer builds. Unset: no preview. |
| `fixture_walk_after` | `MOLY_FIXTURE_WALK_AFTER` | string | developer window | The seconds of app time the fixture walk waits before it starts; unset: it starts at once. |
| `fixture_walk_hold` | `MOLY_FIXTURE_WALK_HOLD` | string | developer window | The seconds the played fixture timeline runs before the walk pushes the joystick to end it; unset: 8 seconds. |
| `fixture_walk_id` | `MOLY_FIXTURE_WALK_ID` | string | developer window | The id of the placed fixture the walk targets; unset: the nearest placed fixture with a player timeline row. |
| `fixture_walk_secs` | `MOLY_FIXTURE_WALK_SECS` | string | developer window | Arms the fixture walk until that many seconds of app time; unset or zero: the walk never starts. |
| `fixture_walk_tap` | `MOLY_FIXTURE_WALK_TAP` | string | developer window | The `x,y` window position of the timeline button the fixture walk taps; unset: the walk stops without tapping. |
| `fixture_walk_via` | `MOLY_FIXTURE_WALK_VIA` | string | developer window | `x,z;x,z` waypoints the fixture walk passes in order before the target; unset: it walks straight to the target. |
| `gate_change` | `MOLY_GATE_CHANGE` | string | developer window | `<gateId>[/<skinId>]:<group>,...`: makes the gate change request with those unit groups; unset: no request from the instrument. |
| `gate_invite` | `MOLY_GATE_INVITE` | string | developer window | `<unit>[,pass]`: makes the gate's reserve request for that unit (optionally granting a pass first); unset: no request from the instrument. |
| `get_resource_autoclose_secs` | `MOLY_GET_RESOURCE_AUTOCLOSE_SECS` | string | developer window | Seconds after a get-resource dialog shows before a tap on its close area; unset: no tap. |
| `get_resource_notice` | `MOLY_GET_RESOURCE_NOTICE` | string | developer window | `type:id:qty[,...]`: calls NoticeCollectItem for each once the get-resource inputs are ready; unset: none. |
| `get_resource_notice_secs` | `MOLY_GET_RESOURCE_NOTICE_SECS` | string | developer window | Seconds after the inputs are ready before the get-resource instrument fires; unset: 0. |
| `get_resource_reward_dialogs` | `MOLY_GET_RESOURCE_REWARD_DIALOGS` | string | developer window | Comma-separated reward dialog ids (251, 263, 452) shown as their delivery caller shows them; unset: none. |
| `get_resource_sketch` | `MOLY_GET_RESOURCE_SKETCH` | string | developer window | The blueprint id whose sketch result dialog is shown once the inputs are ready; unset: none. |
| `harvest_autoplay` | `MOLY_HARVEST_AUTOPLAY` | string | developer window | Instrument: a comma-separated plan of harvest kinds (`wood`, `mineral`, `plant`, `other`, `toolbox`, `driftage`, each optionally `kind:N` for N swings); it walks to each object, presses the action button and gathers the drops, writing the player's own inputs. Unset: no autoplay. |
| `harvest_mock_material_possession` | `MOLY_HARVEST_MOCK_MATERIAL_POSSESSION` | string | developer window | The material quantity the harvest possession mock seats until a reply carries one; unset: 0. |
| `harvest_mock_phenomena` | `MOLY_HARVEST_MOCK_PHENOMENA` | string | server | States the document's learned phenomena from comma-separated `mysekaiPhenomenaId:obtainedAt` entries; a malformed entry stops the start with a panic. Unset: the document's own rows. |
| `harvest_mock_tools` | `MOLY_HARVEST_MOCK_TOOLS` | string | server | States the document's tools from comma-separated `toolId:quantity:durability` entries; a malformed entry stops the start with a panic. Unset: the document's own tools. |
| `harvest_summary_mock_player_total_exp` | `MOLY_HARVEST_SUMMARY_MOCK_PLAYER_TOTAL_EXP` | string | developer window | The mock player total experience the harvest summary panel starts from; a value that is not an int is logged and ignored. Unset: 100000. |
| `harvest_summary_mock_trip_exp` | `MOLY_HARVEST_SUMMARY_MOCK_TRIP_EXP` | string | developer window | The mock experience the server's replies add over one harvest trip; a value that is not an int is logged and ignored. Unset: 120. |
| `home_action_autoplay` | `MOLY_HOME_ACTION_AUTOPLAY` | string | developer window | `craft\|canvas\|sketch\|screen`: walks to the nearest craft tool and runs that home action; unset: no autoplay. |
| `home_action_autoplay_after` | `MOLY_HOME_ACTION_AUTOPLAY_AFTER` | string | developer window | Seconds after the joystick opens before the home action autoplay starts; unset: 2. |
| `home_action_autoplay_skip` | `MOLY_HOME_ACTION_AUTOPLAY_SKIP` | string | developer window | Seconds after the home action request before skip is pressed, when that long into the sequence; unset: no skip. |
| `home_action_sketch_blueprint` | `MOLY_HOME_ACTION_SKETCH_BLUEPRINT` | string | developer window | The blueprint id the home action autoplay sketches; unset: no target. |
| `info_mock_access_permission` | `MOLY_INFO_MOCK_ACCESS_PERMISSION` | string | developer window | The info panel's mock access permission (all/friend_only/reject/review); unset: all. |
| `info_mock_fixture_joint_put_limit` | `MOLY_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT` | string | developer window | The info panel's mock joint fixture put limit; unset: 10. |
| `info_mock_fixture_put_limit` | `MOLY_INFO_MOCK_FIXTURE_PUT_LIMIT` | string | developer window | The info panel's mock fixture put limit; unset: 20. |
| `inventory_recycle_confirm_secs` | `MOLY_INVENTORY_RECYCLE_CONFIRM_SECS` | string | developer window | Seconds after the inventory recycle confirmation shows before its OK is tapped; unset: no tap. |
| `joystick_autosmoke_secs` | `MOLY_JOYSTICK_AUTOSMOKE_SECS` | string | developer window | Injects the scripted joystick touch cycle until that many seconds of app time; unset or zero: no injection. |
| `learn_phenomena_autotap_secs` | `MOLY_LEARN_PHENOMENA_AUTOTAP_SECS` | string | developer window | Seconds after the learn-phenomena open animation before the thumbnail and then the close area are tapped; unset: no taps. |
| `library_open` | `MOLY_LIBRARY_OPEN` | string | developer window | Opens the content library when its talks are ready; unset: it stays closed. |
| `library_qa_out` | `MOLY_LIBRARY_QA_OUT` | path | developer window | Writes the content library's QA telemetry JSON to this file every 0.5 s; unset: no file is written (the browser export still works). |
| `menu_mock_crystal_shop_allowed` | `MOLY_MENU_MOCK_CRYSTAL_SHOP_ALLOWED` | string | developer window | Whether the menu's mock allows the crystal shop; unset: true. |
| `menu_mock_sketch_available` | `MOLY_MENU_MOCK_SKETCH_AVAILABLE` | string | developer window | Whether the menu's mock makes sketch available; unset: true. |
| `menu_mock_stamina_boost` | `MOLY_MENU_MOCK_STAMINA_BOOST` | string | server | Overlays this boost stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value. |
| `menu_mock_stamina_enhance` | `MOLY_MENU_MOCK_STAMINA_ENHANCE` | string | server | Overlays this enhance stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value. |
| `menu_mock_stamina_max` | `MOLY_MENU_MOCK_STAMINA_MAX` | string | server | Not applied: the stamina gauge maximum is the master `maxStamina`; when set, a warning says so. Unset: no warning. |
| `menu_mock_stamina_normal` | `MOLY_MENU_MOCK_STAMINA_NORMAL` | string | server | Overlays this normal stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value. |
| `menu_mock_total_exp` | `MOLY_MENU_MOCK_TOTAL_EXP` | string | server | Overlays this total experience on the default document's `userMysekaiGamedata.totalExp` and clears its rank, so the rank follows the experience; a value that is not an integer is logged and the document keeps its value. Unset: the document's own value. |
| `menu_mock_visiting` | `MOLY_MENU_MOCK_VISITING` | string | developer window | The menu's mock visiting state (true/1 or false/0); unset: false. |
| `music_mock_owned_records` | `MOLY_MUSIC_MOCK_OWNED_RECORDS` | string | server | The music records the mock user owns, comma-separated record ids obtained at the server clock; an id that is not an integer, or named twice, stops the start with a panic. Unset: the user owns no record. |
| `notice_collect_autoplay` | `MOLY_NOTICE_COLLECT_AUTOPLAY` | string | developer window | How many scripted NoticeCollectItem calls the collect notice sends two seconds after its cells are ready (a non-number sends all); unset: none. |
| `npc_entry_probe` | `MOLY_NPC_ENTRY_PROBE` | string | developer window | A `;`-separated script of gate and cut-scene entry steps (`SECONDS:OP[:UNIT[:ARG]]`) driven in developer builds, one log line per step. Unset: no steps run. |
| `npc_harness_dir` | `MOLY_NPC_HARNESS_DIR` | path | developer window | The acceptance harness directory: cases are read from `<dir>/in/<name>.json` and results written to `<dir>/out/<name>.json`. The harness drivers stop with a panic naming the key when it is unset. |
| `npc_hfsm_scope` | `MOLY_NPC_HFSM_SCOPE` | string | developer window | The units whose NPC machine the controller drives (unit ids separated by commas or spaces, or `all`). Unset: the controller has no scope; on a jp source of version 7 or later it enables itself for every unit. |
| `npc_hfsm_trace` | `MOLY_NPC_HFSM_TRACE` | string | developer window | Writes the NPC machine's per-decision, per-transition and per-recovery log lines; `1`, `true`, `yes` or `on` switch them on. Unset or any other value: only the constructor and report lines. |
| `oc_model` | `MOLY_OC_MODEL` | path | developer window | A VRM or glTF model file the player wears from the start on native builds; an empty value is ignored. Unset: the player keeps the server's avatar body. |
| `oc_unit` | `MOLY_OC_UNIT` | string | developer window | The unit id of the character body the player wears from the start on native builds; a value that is not a unit id is refused and logged. Unset: the server's avatar body. |
| `offline_home_level` | `MOLY_OFFLINE_HOME_LEVEL` | string | developer window | The home site's offline level a native start uses; unset: the site's saved level. |
| `option_mock_user_config` | `MOLY_OPTION_MOCK_USER_CONFIG` | string | developer window | `<true\|false>,<all\|id_search\|reject>`: the mock user config the option screen reads; unset: the user config is null. |
| `partvoice_probe_secs` | `MOLY_PARTVOICE_PROBE_SECS` | string | developer window | After this many seconds of app time a one-shot composed part-voice segment is injected through the real talk chain to exercise the playback, missing-cue, fixture and gate-reject arms; a value that is not a number is warned about and turns the probe off. Unset: no injection. |
| `pick_npc_tap_secs` | `MOLY_PICK_NPC_TAP_SECS` | string | developer window | Injects a tap on each present character's screen position in turn until that many seconds of app time; unset: no taps. |
| `player_autowalk_diagonal` | `MOLY_PLAYER_AUTOWALK_DIAGONAL` | string | developer window | When positive, the autowalk direction is the normalized diagonal instead of +X; unset: +X. |
| `player_autowalk_secs` | `MOLY_PLAYER_AUTOWALK_SECS` | string | developer window | Composes a fixed walk (then dash) input for that many seconds after startup; unset: no autowalk. |
| `player_talk_autotap_secs` | `MOLY_PLAYER_TALK_AUTOTAP_SECS` | string | developer window | Instrument: for `<until>` or `<from>:<until>` seconds of app time, a synthetic tap on an on-screen NPC with a general-talk candidate is fed through the real pick and talk chain every few seconds. Unset or not a number: no taps. |
| `player_talk_stack_tap_secs` | `MOLY_PLAYER_TALK_STACK_TAP_SECS` | string | developer window | A window (`<until>` or `<from>:<until>` seconds) in which the stack head's Talk button request is written for the character under it; unset: the probe is off. |
| `procedural_debug` | `MOLY_PROCEDURAL_DEBUG` | string | developer window | Prints the first particle whose position, velocity or axis differs from the native row; unset: no debug output. |
| `rank_change` | `MOLY_RANK_CHANGE` | string | developer window | `<prev>,<current>`: delivers a rank change before the first site loads; unset: no rank change. |
| `room_level` | `MOLY_ROOM_LEVEL` | string | developer window | The room level a native start uses; unset: the site's saved level. |
| `scene_content` | `MOLY_SCENE_CONTENT` | string | developer window | `compact` or `full`: how much of a site's content is loaded at a native start; unset: compact. |
| `screenshot_dir` | `MOLY_SCREENSHOT_DIR` | path | developer window | The directory a requested frame capture is written to. Unset: captures go to `screenshots`. |
| `seed` | `MOLY_SEED` | string | developer window | The world seed, a decimal u64: every random stream draws a fixed sequence from it, so a run can be repeated. Unset: each draw takes fresh platform random bytes. |
| `shadow_probe_interval` | `MOLY_SHADOW_PROBE_INTERVAL` | string | developer window | The frames between shadow-map depth accounts on native (at least 1); unset: no native accounts run. |
| `silhouette_probe_interval` | `MOLY_SILHOUETTE_PROBE_INTERVAL` | string | developer window | The native interval, in replayed frames, between silhouette stencil read-backs; the first read-back comes on frame 10 and a value of 0 is ignored. Unset: no native read-back. |
| `site` | `MOLY_SITE` | string | developer window | The site type a native start enters; unset: the saved preferred site, else the home site. |
| `sitemap_autoclick_secs` | `MOLY_SITEMAP_AUTOCLICK_SECS` | string | developer window | Seconds before the site map instrument opens the map and taps a site's icon; unset: no auto click. |
| `sitemap_autoclick_site` | `MOLY_SITEMAP_AUTOCLICK_SITE` | string | developer window | The site type the site map instrument taps; unset: no auto click. |
| `sitemap_auto_unlock_secs` | `MOLY_SITEMAP_AUTO_UNLOCK_SECS` | string | developer window | Seconds before the site map instrument runs the first locked icon's open animation once; unset: nothing. |
| `site_extension_dev` | `MOLY_SITE_EXTENSION_DEV` | string | developer window | Plays one site expansion clip from these `;`-separated `key=value` pairs through the developer hook; unset: the hook plays nothing. |
| `site_move_landing_draw` | `MOLY_SITE_MOVE_LANDING_DRAW` | string | developer window | Replaces the site move's landing draw with this value (the failed landing is one of the draws); unset: a random draw. |
| `site_tour_route` | `MOLY_SITE_TOUR_ROUTE` | string | developer window | The smoke tour's route: a comma-separated list of site types visited in that order instead of the master-row order; an unsupported site type stops the start with a panic. Unset: master-row order. |
| `site_tour_secs` | `MOLY_SITE_TOUR_SECS` | string | developer window | The smoke-tour instrument's dwell time in seconds: the game loads each site in turn unattended and exits cleanly after the last one; a value of 0 or not a number counts as off. Unset: no tour. |
| `step_item` | `MOLY_STEP_ITEM` | string | developer window | `<bundle>\|<file>` of a step item: once the avatar is wired the step item sequence runs once to check the service alone; a value without `\|` is logged as an error. Unset: no sequence. |
| `talk_auto_advance` | `MOLY_TALK_AUTO_ADVANCE` | string | developer window | Set to `1`: a talk line advances as soon as its own voice has played out, without a tap, for unattended recordings. Any other value or unset: a tap advances the line. |
| `talk_tap_at_secs` | `MOLY_TALK_TAP_AT_SECS` | string | developer window | Smoke input: a comma-separated list of app times in seconds at which a tap on the talk window is set synthetically; parts that are not numbers are skipped. Unset or empty: no taps. |
| `talk_voice_probe_lines` | `MOLY_TALK_VOICE_PROBE_LINES` | string | developer window | The number of voice lines in the synthetic talk segment; a value that is not a positive integer is warned about and the default is used. Unset: 3. |
| `talk_voice_probe_secs` | `MOLY_TALK_VOICE_PROBE_SECS` | string | developer window | After this many seconds of app time a one-shot synthetic talk segment with voice lines is injected through the real talk chain, once no conversation plays; a value that is not a number is warned about and turns the probe off. Unset: no injection. |
| `visitor_units` | `MOLY_VISITOR_UNITS` | string | developer window | Replaces the roster of visitors the home site boots with: a comma-separated list of unit ids; parts that are not numbers are skipped, and an empty list leaves the gate rows. Unset: the roster comes from the gate rows. |
| `walk_bake_threads` | `MOLY_WALK_BAKE_THREADS` | string | developer window | How many row bands the walk-field collider bake may spread over, clamped to 1..8 (0 and 1 keep it on one thread); a value that is not a number counts as unset. Unset: the machine's available parallelism, capped at 8. |
| `walk_probe` | `MOLY_WALK_PROBE` | string | developer window | QA probe: for every walk-field generation, logs the door locators' static path queries and the agent path for each `from>to` pair given here (x,z coordinates). Unset: no probe. |
| `window_hidden` | `MOLY_WINDOW_HIDDEN` | string | developer window | Starts a native run with the window hidden for headless batches; any value hides the window. Unset: the window is shown. |

## Before the file

Two environment variables decide where the file is. They are read only from the environment and are not keys of the file.

| Variable | Description |
|---|---|
| `MOLY_DATA_DIR` | The data directory. `moly.toml` sits in it unless `MOLY_CONFIG` names another file. See the [launcher documentation](../crates/app/moly-launcher/README.md). |
| `MOLY_CONFIG` | The configuration file. When set and not empty, it replaces `moly.toml` in the data directory. |

## Page parameters (browser version)

| Parameter | Description |
|---|---|
| `server` | Address of the server to connect to. Defaults to the public server. |
| `renderer=webgl2` | Uses the WebGL2 backend |
| `player_uid`, `player_region`, `player_api` | Inputs for fetching player data |

Each browser stores a random user ID in its local storage.
