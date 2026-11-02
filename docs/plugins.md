# Plugins

Plugins extend the native game with Luau scripts and, when a manifest says so,
with a native provider behind a seam. This page describes what a plugin can see
and do: the `moly` interface, every director step it may run, the sections it
may observe, the events it receives, and the environment switches a recorder or
a plugin author uses.

A plugin is a directory with a `plugin.toml` in the game's `plugins` directory
(beside the executable) or in the data directory's `plugins` directory. The
profile `plugins/profile.toml` in the data directory names the plugins it
enables; without one the profile is empty and no plugin loads. The full plugin
set is checked at startup (version requirements, dependencies, conflicts, seams)
and a set that cannot start stops the game with its reasons in the log.

```lua
-- main.luau: log an observation, walk the player, keep a counter.
moly.log("plugin", moly.id, "loaded")
moly.on("tick", function(dt) end)
moly.on("site_entered", function(event) moly.log("on", event.site_type) end)
moly.spawn(function()
    local seen = moly.store.get("walks") or 0
    local where = moly.observe({ "site" })
    moly.log("site", where.site.site_type, "fixtures", #where.site.occupied)
    local walked = moly.act({ ["do"] = "player.walk_to", to = { point = { 0, 0 } } })
    if walked.ok then
        moly.store.set("walks", seen + 1)
    else
        moly.log("refused:", walked.reason)
    end
end)
```

## The Luau interface

Each plugin runs in its own sandboxed Luau state. The standard library is the
coroutine, table, string, utf8, bit, math, buffer and vector libraries; `io`,
`os`, `debug` and `package` are absent, globals are read-only (`moly` cannot be
replaced), memory is limited per plugin and each frame has an instruction
budget. `loadstring` is disabled: scripts come from the files of the plugin
directory only.

| Member | What it does |
|---|---|
| `moly.id` | The plugin's id from its manifest. |
| `moly.config` | The plugin's resolved settings: the manifest's `[config]` table under the profile's overrides. |
| `moly.log(...)` | Writes a line to the game log (`[plugins] <id>: ...`). Lines written while the script loads come out on its first frame. |
| `moly.on(name, fn)` | Registers a handler: `"tick"` (called with the frame's delta in seconds), an event kind (called with the event), or `"*"` (called with every event). |
| `moly.spawn(fn, ...)` | Starts a task: a coroutine resumed once a frame that may wait. |
| `moly.observe(what?)` | The world as structured data, by section (below). |
| `moly.act(step)` | Runs one director step and waits for it (tasks only). `{ ok = true, value = <note> }`, or `{ ok = false, reason = "<refusal>" }`; a step that never ran is refused at once, one that ended without a note answers `value = nil`. |
| `moly.wait(seconds)` | Waits inside a task. |
| `moly.now()` | Seconds of game time since the plugin started. |
| `moly.random()` | A number in [0, 1) from the machine's own source at each call (the sandboxed `math.random` repeats a run's seed). |
| `moly.store.get(key)`, `moly.store.set(key, value)` | The plugin's own saved data, one JSON object at `plugins/store/<id>.json`, written at the end of the frame a key changed in. `set(key, nil)` removes the key. |
| `require("name")` | Loads `<plugin dir>/name.luau` once and returns what it returns; names are relative paths inside the plugin directory and paths that leave it are refused. |

The host functions exist only during a frame: a handler or a task that keeps a
reference to one cannot call it later. A handler or a task that raises an error
or exceeds the instruction budget counts as one failing frame; a plugin failing
three frames in a row is stopped, with the reason in the log and in
`moly.plugins`.

### Control lease and hand steps

The steps that drive the player through the virtual hand belong to one owner at
a time: `player.*`, `camera.*`, `tap`, `ui.tap` and `dialog.answer`. A human is
above the director and a plugin is below it. While a director script runs, a
plugin's hand step is refused (`a director script holds the hand`); while
another plugin's hand step holds the hand, that step is refused (`the hand is
held by another plugin's step`). The hand step that starts first holds the hand
until its track ends. When human input arrives (a key, a mouse button, a touch,
a gamepad button or stick), the hand step that holds the hand is aborted and
fails with `human input took control`. Every other step is not a hand step.

## Director steps

`moly.act` takes exactly the step objects a director script takes. A script is
`{ "name"?, "exit"?, "steps": [...] }`, each step an object with `do` and its
fields, plus `label` (the step's name in the log, the action's name by default),
`timeout` (seconds before the step fails; it replaces the step's own default)
and `optional` (a failure is logged and the script goes on). Unknown actions and
unknown fields are refused before anything runs.

The `moly.director.schema` method answers the schema of every step and
condition; `moly.director.examples` the bundled example scripts. A macro step
expands into several steps; the common fields apply to each of them.

### Wait and flow

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `wait` | `secs` | Waits `secs` game seconds. | Ends when the time has passed. Timeout: none. |
| `wait_until` | `until` (condition) | Waits until the condition holds. | Ends when it holds; fails at the timeout (30 s default). |
| `assert` | `that` (condition) | Checks a condition once. | Ends at once when it holds; fails with the condition's reading when it does not. |
| `log` | `text` | Writes `text` to the run's log. | Ends at once; the note is `text`. |
| `where` | `of` (targets; by default none) | Logs the player's position and each target's. | Ends at once with the positions. |
| `capture` | | Saves the next rendered frame. | Ends at once. |
| `parallel` | `tracks` (lists of steps), `join` (`all` \| `any`, default `all`) | Runs the tracks side by side. | `all`: every track ended. `any`: one track ended, the others are stopped. Fails when a track fails (with `any`, the others are stopped). |
| `repeat` | `times` (integer), `steps` | Runs the nested steps `times` times. | Ends when the last round has ended; `times: 0` runs nothing. |

### Player

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `player.walk_to` | `to` (target), `within` (default 0.3) | Holds the joystick along a path on the navigation field, re-planning after a stall or a moved goal. | Ends within `within` metres of the target; fails at the timeout (60 s default) or with the walk's reason (no route, the walk stopped short). |
| `player.interact` | `target`, `button` (one of the button types), `stop` (default 0.2) | Walks until the button heads the action-button stack (tapping the change-target button when it is stacked behind another) and presses it. | Ends once the button is pressed; fails at the timeout (60 s default) or with the walk's reason. |
| `player.talk_to` | `unit`, `stop` (default 0.8) | Macro: `player.interact` on the character's Talk button, then `wait_until "talk_active"`. | Ends when a conversation is playing; fails as `player.interact` does, or at the 10 s wait for the conversation. |
| `player.stick` | `stick` (`[x, y]` ratio inside the unit circle, x right, y up), `secs` | Holds the joystick at the ratio for `secs`. | Ends after `secs`; fails when another step's hand has the joystick finger. A ratio outside the unit circle is refused when the step is parsed. |
| `player.release` | | Lifts the joystick finger. | Ends at once. |
| `player.wear` | `unit` | Chooses a character's body as the player's (the body is worn when it has loaded). | Ends once the choice is made. |
| `player.wear_avatar` | | Chooses the avatar body. | Ends once the choice is made. |
| `tap` | exactly one of `at` (`[x, y]` logical pixels) and `at_fraction` (`[fx, fy]` fractions of the window) | Taps the screen. | Ends at once; fails when there is no primary window. |
| `ui.tap` | `view` (a document key), `path` (a unique node path suffix or `@<id>`) | Taps the centre of a node of a shown UI view. | Ends at once; fails when the view is not shown (the error names the views that are) or the node is not found. |
| `dialog.answer` | `ok` (default `true`) | Taps the shown common dialog's OK (`true`) or cancel button; a sub-window dialog that has no common button (the learn-phenomenon one) is answered with the back key its own class takes. | Ends at once; fails at the timeout (10 s default) when no dialog is shown. |
| `key.back` | | Presses the back key once. | Ends at once; fails when the back key is not accepted now. |
| `player.gather` | `gatherable` (a harvest object UID or `"nearest"`), `kind` (narrows `"nearest"` only), `collect` (default `true`) | Harvests one gatherable through the harvest button, then walks over the drops it leaves with `collect`. | Ends when the harvest answers; fails at the timeout (120 s default) or with the refusal. |
| `player.craft` | `blueprint`, `workbench` (a fixture UID or master id; `false` for none; by default the placed workbench nearest the player) | Macro: an interaction with the workbench's craft button, then the blueprint's choice, the apply button, the confirm's OK and the result's close on the open craft screen. | Ends when the craft answers; fails at the timeouts (60 s each) or with the refusal. |
| `player.deliver` | `party` (by default the first party in session), `hold` (seconds above 0, by default until nothing is left to deliver) | Presses the birthday party's delivery button, holds it and releases it. | Ends when the delivery answers; fails at the timeout (180 s default) or with the named refusal (no party in session). |
| `player.warp_to` | `to` (`[x, y, z]`) | Stands the player at a world point at once. | Ends at once; fails when no player is on the site. |

### Characters

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `npc.take` | `unit` | Cancels the character's objective and holds its AI for this run. | Ends at once; fails when the unit has no character. |
| `npc.walk_to` | `unit`, `to`, `within` (default 0.3), `rests` (default `false`) | Walks a taken character there with its own navigation (`rests` keeps the walk law's rest stops). | Ends on arrival or within `within` (the character stops there); fails at the timeout (60 s default), with the walk's reason, or when the character is not taken. |
| `npc.release` | `unit` | Gives the character back to its AI. | Ends at once; fails when this run has not taken it. |
| `npc.face` | `unit`, `to`, `secs` (default 1.0) | Turns a taken, standing character toward a target over `secs`. | Ends when the turn has finished; fails at the timeout (`secs` + 5), when the character is not taken or is gone, or with the turn's reason. |
| `npc.emote` | `unit`, `name` (an emoticon such as `fx_emote_001`), `secs` (default 2.0), `use_root` (default `false`) | Shows an emoticon over a character. | Ends after `secs`; fails when the emoticon cannot be shown. |
| `npc.use` | `unit`, `fixture` (a placement UID or a master id, the nearest instance), `activity`, `variant` (1-based; the first of each by default), `rests` (default `false`) | Plays one of the character's authored no-talk activities at a placed fixture through the furniture owner its AI uses. | Ends when the activity has completed; fails at the timeout (300 s default), when the character is not taken, or when the activity is cancelled or fails. |
| `npc.tweet` | `unit`, `tweet`, `secs` (default 3.0) | Shows a tweet of the master over a taken character. | Ends after `secs` (the tweet is hidden); fails when the character is not taken or the tweet cannot be shown. |
| `npc.talk` | `unit`, `talk` | Hands the character to its AI with a talk to open: its next decision builds the talk (the talk's fixture drawn, the other speakers drafted) instead of drawing one. | Ends when the talk is built; fails at the timeout (60 s default) or when the talk is refused. |
| `npc.warp_to` | `unit`, `to` (`[x, y, z]`) | Stands a character at a world point at once; its AI makes no decision until released. | Ends at once; fails when the unit has no character. |
| `npc.play_talk` | `unit`, `talk`, `fixture`? | Plays a conversation at once, without the cast's gathering. A fixture conversation needs `fixture`: a placed fixture's UID, or a master id (the nearest placed instance). | Ends at once when it starts; fails at the timeout (20 s default) or when it is refused. |

### Companions

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `companion.spawn` | `unit`, `at` (`[x, y, z]`) | Puts a companion of the unit's character at a world point; it walks to the point when its seat is off it. | Ends once it is shown and standing; fails at the timeout (60 s default), when a companion of the unit is already spawned or being spawned, or when it cannot be spawned. |
| `companion.walk_to` | `unit`, `to`, `within` (default 0.3, capped by the walk law's limit) | Walks the companion there. | Ends within `within`; fails at the timeout (60 s default), when no companion is spawned, or when it was removed while walking. |
| `companion.face` | `unit`, `to` (a world point or `"player"`), `secs` (default 0.4) | Turns the companion to face the point over `secs`. | Ends when the turn has finished; fails at the timeout (`secs` + 5) or when no companion is spawned. |
| `companion.emote` | `unit`, `name`, `secs` (default 2.0), `use_root` (default `false`) | Shows an emoticon over the companion. | Ends after `secs`; fails when no companion is spawned or the emoticon cannot be shown. |
| `companion.motion` | `unit`, `motion` | Plays one motion of the shared motion library on the companion's own body once (its start, loop and end segments, then its idle). | Ends when the motion has played; fails at the timeout (60 s default) or when no companion is spawned. |
| `companion.expression` | `unit`, `eye`, `mouth`, `reset` (default `false`) | Sets the eye and/or mouth pattern of the companion's body, or returns it to the unit's default face. | Ends at once; fails when no companion is spawned or the patterns are refused. `reset` takes neither `eye` nor `mouth`, and without `reset` one of them is required. |
| `companion.balloon` | `unit`, exactly one of `text` and `tweet`, `secs` (default 3.0) | Shows a tweet balloon over the companion and hides it when the step ends. | Ends after `secs`; fails at the timeout (`secs` + 5), when no companion is spawned, or when the tweet master has no such row. |
| `companion.despawn` | `unit` | Removes the companion. | Ends once it is gone; fails at the timeout (10 s default) or when none is spawned. |
| `companion.build` | `unit`, `fixture`, `texture`, `near` (`[x, y, z]`; by default in front of the companion), `at` (the cell's own, or the step fails instead of taking the nearest legal tile), `direction` (0 to 3), `swings` | Puts an owned fixture near the point and has the companion hammer it from a ghost to solid. | Ends when the build has finished; fails at the timeout (120 s default) or with the build's reason. Takes one of `near` and `at`. |
| `companion.use` | `unit`, one of `fixture` (the nearest placed copy of the master id) and `uid`, `activity`, `variant` | Has the companion walk to the fixture's action point for its character's action and play it. | Ends when the action has finished; fails at the timeout (120 s default) or with the reason. |
| `companion.gather` | `unit`, `gatherable` (a harvest object UID or `"nearest"`), `kind` (`wood` or `mineral`, with `"nearest"`) | Has the companion walk beside a wood or mineral harvest object and swing the player's tool for it until it is harvested; each hit is a hit of the player's harvest. | Ends after the last swing; fails at the timeout (120 s default) or with the reason. |

### Camera and view

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `camera.drag` | `by` (`[dx, dy]` logical pixels, x right, y down), `secs` (default 0.5), `from` (fractions of the window; default above the player) | Drags one finger through the gesture layer and the camera's own input law. | Ends when the drag has finished; fails when there is no primary window or the touch is refused. |
| `camera.zoom` | `by` (pixels; positive zooms in), `secs` (0.5 by default) | Turns the wheel by `by` pixels spread over `secs`. | Ends when the amount has been turned; fails when the wheel is refused. |
| `view.set` | `hud`, `player` (booleans; each changes only when given) | Sets the clean view: `hud: true` hides every UI view but the talk window, `player: true` hides the player's own body. | Ends at once; the note names the state (`view: hud hidden, player shown`). Refused when it gives neither switch. It is not a hand step. |
| `camera.free` | `position` (`[x, y, z]`), `target` (`[x, y, z]`), `fov` (degrees in 5..=120, default 45) | Holds the field camera at a world pose (a screenshot's eye). | Ends at once; a field of view outside 5..=120 is refused when the step is parsed. |
| `camera.talk_lens` | `distance` (1.9), `height` (0.8), `target` (0.47), `fov` (35), `swing` (0.5), `unit`, `wide` (`true`), `wide_hold` (2.5), `glide` (2.5), `push` (0.12), `clear` (`true`) | Frames conversations with the talk lens, following the speaker: a line spoken toward the player is framed from the front, other lines from a three-quarter view. The eye eases toward each new framing at `glide` per second (0 cuts) and closes in by `push` of its distance while the framing holds. With `clear`, a placed fixture standing between the eye and a cast member fades out with the fixture dither until it no longer blocks the view. | Ends at once. |
| `camera.talk_lens_off` | | Turns the talk lens off. | Ends at once. |

### Layout

The layout steps act on the site's layout through the layout editor's own draft
and save path: a put meets the editor's put rules, a save runs the editor's save
checks and the server's housing layout call, and a step ends once the layout has
reloaded. One layout save is out at a time; a step that finds another save in
flight refuses. Cells are the layout grid's cells `[x, z]`, the frame
`moly.observe` `site.occupied` and `moly.layout.view` report. `face` is the
rotation: `front` the master's own facing, `left` and `right` its quarter turns,
`back` the one that puts a door toward +z, the side the default camera starts
from.

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `layout.place` | `fixture`, `texture`, `center` (`[x, y, z]`), `rotate` (0 to 3 quarter turns, default 0), `decide` (default `true`; `false` ends the step once the fixture is on the tile, still selected, with nothing decided or saved), `save` (default `true`) | Puts one owned fixture through the editor's own commands, the objects `moly.layout.command` takes: enter, select the catalog row with a placeable copy, turn it, move it onto the tile, decide it and save. | Ends when the editor has done so; fails at the timeout (120 s default) with the editor's put status and feedback, or when no owned copy is placeable. The editor stays open afterwards; `rotate` above 3 is refused when the step is parsed. |
| `layout.free` | `on` (boolean) | Sets the site's free layout mode in the local document: on, a saved home site layout may hold no house (the start house is still placed on a first visit), the house and the gates can be stored, any number of houses and gates may stand, and a placement no longer needs an owned copy. | Ends once the game holds the mode; the note is `free layout on` (`off`). Refused when no site is active or the local document cannot be written. |
| `layout.clear` | `keep` (`["house", "gate"]`; default none) | Stores the site's fixtures and saves the site once. Without the free layout mode the house and the gates stay placed, as the game keeps them; with it `keep` names the ones that stay. | Ends once the layout has reloaded; the note counts the stored and the kept fixtures. Refused when the layout editor is open, the site is loading or changing, another layout save is in flight, or a save check or the server refuses. Anything but `house` and `gate` in `keep` is refused when the step is parsed. |
| `layout.put` | `fixture`, `texture` (default 1), `at` (`[x, z]`), `y` (default 0), `face` (default `front`) | Places one fixture with its minimum corner (after the rotation) on the cell and saves the site. | Ends once the layout has reloaded; the note names the fixture and its cells (`fixture <id> placed as <uid> at (x0,z0)..(x1,z1) face <face>`). Refused with the reason: `out of the floor`, `overlaps <uid>`, `not owned` (without the free layout mode), `wrong layer`, `unavailable zone`, `no room above`, `the fixture's box leaves the grid`, `the fixture is not in the master`, or the save's refusal (a save check, the server, the layout not loading). |
| `layout.put_many` | `items` (one or more items with `layout.put`'s fields) | Places every item, all or nothing, in one save. | Ends once the layout has reloaded. A refusal names the item: `item <index>: <reason>`, and `item <i>: overlaps <uid>` (the uid of an earlier item, or of a fixture already there). An empty list is refused when the step is parsed. |
| `layout.remove` | exactly one of `uid` (a placement UID) and `at` (`[x, z]`, a cell the fixture covers) | Stores one placed fixture and saves the site once. | Ends once the layout has reloaded. Refused when no fixture of the site is named, for the house and the gates without the free layout mode, and as `layout.put` is for the site's state or a refused save. Giving neither or both fields is refused when the step is parsed. |

### Site and weather

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `site.switch` | `site` (a site type such as `first_floor`) | Macro: waits for the entry's control gate (`wait_until "control_open"`), then requests the move. | Ends once the move has landed on the site; fails at the timeouts (30 s and 60 s), when the source has no move for the pair (the request is dropped), or when the move ends elsewhere. |
| `site.enter_room` | | Macro: waits for `control_open`, walks to the house door's entry box, presses its button, waits for the move to start and to land in a room. | Ends on the room; fails at the timeouts (30 s, 60 s, 5 s, 60 s) or with a button or walk refusal. |
| `site.go_home` | | Macro: waits for `control_open`, walks to the room door's sensor, presses its button, waits for the move to start and to land at home. | As `site.enter_room`. |
| `weather.set` | `phenomenon` | Asks the server model for the phenomenon (a schedule edit of the current window). | Ends at once; the note names the request. |

### Server

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `server.edit` | `path` and `value`, or `action` and its fields (every other field is the edit) | One edit of the server document (the clock, the stamina, the phenomenon schedule, the gate actions `gate.reserve`, `gate.change` and `sync`, the fixture grant `fixture.grant`), checked by the server model as the server panel's edits are. | Ends when the server answers; fails at the timeout (30 s default) with the model's refusal, when the game has no server link, or when no answer comes. A step with neither a path nor an action is refused when the step is parsed. |

### Effects

| Step | Fields | What it does | How it ends / refuses |
|---|---|---|---|
| `effect.play` | `name` (a particle or sprite emoticon such as `fx_emote_018_single`), `at` (`[x, y, z]`), `secs` (default 2.0) | Shows the emoticon at a world point on an anchor entity of its own and takes it away after `secs`. | Ends after `secs`; fails when the emoticon cannot be shown. |

## Observing

`moly.observe(what?)` answers `moly.observe`'s object: `frame` and the sections
named in `what` (every section when it is omitted). An unknown section name is
refused with the list of sections.

| Section | What it holds |
|---|---|
| `site` | The site (`site_id`, `level`, `site_type`, `category`, `scene`, `indoor`), `free_layout` (whether the site's free layout mode is on; a malformed switch is a string naming the error), `layout_site_id`, `floor` (`layout_id`, `width`, `height`, `depth`), and `occupied`: every placed fixture's `uid` and its cells `min` and `max`. |
| `player` | The player's `entity`, `position`, and state (`null` entity when no player is on the site). |
| `gatherables` | The loaded site's harvest objects: position, remaining hit points, whether harvested, the tool they need, whether the player targets them. |
| `bag` | The materials and items of the client's copy of the user's tables, by id and quantity. |
| `tools` | The owned harvest tools. |
| `recipes` | The craft screen's recipes, read through its own checks. |
| `fixtures` | The owned and the placed fixtures. |
| `characters` | Every character with its AI: `unit`, `entity`, the machine's phase and states, `position`, `facing`, `away`, `action`, `tweeting`, `rotating`, `talking`, `objective`, `executing`. |
| `companions` | Every companion: `unit`, `entity`, `position`, `yaw`, `state` (a failed one names its reason), `seq`, the face it shows while it is set, `fixture_at` while it builds, and `after_edit_tweets` (the tweets its unit tweets when the player finishes editing: id, text, motion, eye, mouth, emoticon). |
| `npc_ai` | The character machines: the reported rows (`report`) and each live machine (`live`: phase, upper and lower state, transitions this frame, the inactive timer, the start flags, the upper weights). |
| `weather` | The current phenomenon id and name. |
| `time` | The game's time (elapsed, scale, paused, real elapsed) and the server clock in milliseconds. |
| `talk` | Whether a conversation is playing and its speaker. |

## Events

The game records an event stream; a plugin's `moly.on(kind, fn)` handlers run
once per event of that kind in the frame's order, and `moly.on("*", fn)` for
every event. Every event carries `seq` (its number in the stream), `frame`,
`time` (game seconds), `kind` and its fields. The newest 4096 events are kept;
an `moly.events` reader that falls behind is told how many it missed.

| Kind | Fields | When |
|---|---|---|
| `tick` | `dt` (seconds) | Not an event: the handler runs once a frame. |
| `weather_changed` | `phenomenon`, `name`, `from` | The phenomenon shown has changed. |
| `site_entered` | `site_id`, `site_type`, `from` | The active site has changed. |
| `talk_started` | `talk_id`, `with_player`, `unit` (with the player) | A conversation has started. |
| `talk_ended` | `talk_id`, `with_player`, `unit` (with the player) | A conversation has ended. |
| `character_arrived` | `unit`, `site_type` | A character has come to the site. |
| `character_left` | `unit`, `site_type` | A character has left the site. |
| `tweet_shown` | `unit`, `tweet_id`, `text`, `entry` (a character's arrival tweet) | A character's tweet has been shown. |
| `item_obtained` | `uid`, `site_id`, `resource_type`, `resource_id`, `quantity`, `rarity` | A harvest object's drop has been collected. |
| `craft_done` | `blueprint_id`, `craft_type`, `target_id`, `texture_id`, `quantity`, `spent`, `first`, `exp` | A craft answered. |
| `craft_refused` | `blueprint_id`, `quantity`, `reason` | A craft was refused. |
| `recycle_done` | `api`, `kind`, `id`, `quantity`, `obtained`, `inventory_revision`, `tools_revision` | A recycle or disassemble answered. |
| `recycle_refused` | `api`, `kind`, `id`, `quantity`, `reason` | A recycle or disassemble was refused. |
| `delivery_started` | `party_id` | A birthday party's delivery has started. |
| `delivery_refused` | `party_id`, `reason`, `have`, `spent`, `code` | A delivery was refused. |
| `delivery_done` | `party_id`, `consumed` | A delivery answered. |
| `server.call` | `seq`, `call` | A call was sent to the server. |
| `server.reply` | `seq`, `call`, `ok`, `refusal` (when refused) | The server answered a call. |
| `server.push` | `kind` | The server sent on its own. |
| `server.link` | `state` (`connecting`, `open`, `refused`, `offline`, `lost`), `server`, `reason` | The connection's state changed. |

## Seams

The game declares four seams a native or Luau provider may replace, listed by
`moly.plugins`: `companion.brain` (what an AI player does next while no human
controls it; target `unit:<id>`), `npc.objective` (a character's next objective;
target `unit:<id>`), `game.tick` (once per simulation frame) and `game.events`
(the event stream). The first two take one provider per target, the last two
any number, run in load order.

## Switches

| Variable | What it does | Honoured by |
|---|---|---|
| `MOLY_DEV` | `1` starts the developer window instead of the game. | Native program |
| `MOLY_ASSET_ROOT` | The resource root directory. | Native program (game, developer window and stage) |
| `MOLY_SERVER_URL` | The server address (`http://host:port` or `https://host`). | Native program (game and developer window) |
| `MOLY_DATA_DIR` | The data directory: saves, settings, logs, and the plugins' `plugins` directory. | Native program (game and developer window) |
| `MOLY_SETTINGS_FILE` | The settings file. | Native program (game and developer window) |
| `MOLY_HIDE_HUD` | `1` hides every HUD, button and dialog view (the talk window stays); `0` or unset shows the game's own interface. Any other value is refused at start by name. | Native program: game, developer window and stage |
| `MOLY_HIDE_PLAYER` | `1` hides the player's own body; `0` or unset shows it. Any other value is refused at start by name. | Native program: game, developer window and stage |
| `MOLY_DIRECTOR_SCRIPT` | A script file that runs at start; with `"exit": true` the program ends when the script does. | Native program: game and developer window |
| `MOLY_WINDOW_HIDDEN` | Starts the native window hidden. | Native program: game and developer window |
| `MOLY_FIXTURE_DIRECTION` | `0` to `3` rewrites the direction of every placed fixture row (a smoke-test knob; an invalid value is refused). | Native program: game and developer window |
| `MOLY_EDIT_AUTOPLAY` | Any value logs the editor's put-rule census when the layout editor opens. | Native program: game and developer window |
| `MOLY_PLAYER_UID`, `MOLY_PLAYER_DATA_FILE`, `MOLY_PLAYER_API`, `MOLY_PLAYER_REGION` | Player-data import at start. | Native program: developer window and stage |

`view.set` and the two hide switches set the same clean view: a recorder can
start with them in the environment and a plugin or a script can change them
while the game runs.

## Keeping this current

The step tables above are checked against `director/schema.json`: a step added
to the schema needs its row here in the same change, and the test
`the_plugin_document_names_every_step` in `moly-devtools` fails naming every
`do` value that has no row. The observe sections, event kinds and environment
switches are kept the same way: when one is added or removed, its row changes
with it.
