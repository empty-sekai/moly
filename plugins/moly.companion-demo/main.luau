-- The companion demo: it reads the player's position with moly.observe, spawns
-- the companion of the configured unit two metres beside the player, waits
-- until the spawn is done, walks the companion to either side of the player
-- three times, then has it build a fixture: the fixture is put on the site as
-- a ghost in front of the companion and the companion hammers it solid.
--
-- Each build ends with a little scene: the companion turns to the player when
-- the player stands near it, tweets one of its after-edit tweets (a random one
-- of its unit's pool: the face, the motion, the emoticon and the text
-- balloon together, then the default face again). A row that names no
-- emoticon plays a random effect of the finale's on the companion instead.
-- Then the companion uses what it built: its character's own action at that
-- fixture (getting on the motorbike, for the default build).
--
-- A `stage` table in the config stages the same story as one scene: the
-- companion appears at `enter` (degrees and distance from the player), walks
-- to `stand` instead of the three rounds, builds at `near`, and each beat
-- (enter, build, finale, use) starts its `cues` (director steps, such as
-- camera moves, run in order beside the beat). Once the player stands the
-- `setup` steps run in order (placing the player and the camera), then
-- `start_delay` seconds pass before the scene.
--
-- Each step goes through moly.act, which runs a director step and resumes the
-- task with `{ ok = true }` or `{ ok = false, reason = ... }`.

local unit = moly.config.unit or 14
local build = moly.config.build or { fixture = 727, texture = 1, swings = 4 }
local face_within = moly.config.face_player_within or 6
local finale_effects = moly.config.finale_effects or { "fx_emote_018_single", "fx_emote_010" }
local stage = moly.config.stage
-- How long the tweet balloon stays up, seconds, and how long the emoticon of
-- the row plays.
local BALLOON_SECS = 5
local EMOTE_SECS = 2
-- The after-edit tweet and the effect the last finale picked.
local picked_tweet, picked_effect = nil, nil

-- A point `distance` metres from `from` along a direction `degrees` from +x.
local function offset(from, degrees, distance)
	local radians = degrees * math.pi / 180
	return { from[1] + math.cos(radians) * distance, from[2], from[3] + math.sin(radians) * distance }
end

-- Runs one step and logs its outcome.
local function run(what, step)
	local outcome = moly.act(step)
	if outcome.ok then
		moly.log(what, "done")
	else
		moly.log(what, "failed:", tostring(outcome.reason))
	end
	return outcome.ok
end

-- Runs one step of a task of its own: the step runs beside the task that
-- started it, and its outcome is logged when it ends.
local function beside(what, step)
	moly.spawn(function()
		run(what, step)
	end)
end

-- The player's position, or nil before the player stands on the site.
local function player_at()
	return moly.observe({ "player" }).player.position
end

-- Waits until the player has stood on the same spot for a second: the site
-- places the player once its layout is in.
local function wait_for_player()
	local last = nil
	while true do
		local at = player_at()
		if at ~= nil and last ~= nil and at[1] == last[1] and at[3] == last[3] then
			return at
		end
		last = at
		moly.wait(1)
	end
end

-- Starts the stage's cues of `beat` on a task of their own, one after the
-- other.
local function cue(beat)
	local steps = stage and stage.cues and stage.cues[beat]
	if steps == nil then
		return
	end
	moly.spawn(function()
		for index, step in steps do
			run("cue " .. beat .. " " .. index .. " " .. tostring(step["do"]), step)
		end
	end)
end

-- Walks the companion `distance` metres from the player along `degrees`.
local function walk_beside(what, degrees, distance)
	local player = player_at() or wait_for_player()
	return run(what, { ["do"] = "companion.walk_to", unit = unit, to = offset(player, degrees, distance) })
end

-- The companion's entry of moly.observe, or nil before it stands.
local function companion_row()
	for _, row in moly.observe({ "companions" }).companions do
		if row.unit == unit then
			return row
		end
	end
	return nil
end

-- One random entry of `list`, not the one the last pick gave.
-- `moly.random` draws afresh on every call; `math.random` repeats a run's
-- seed, so a replayed run would pick the same entries.
local function random_entry(list, last)
	local index = math.floor(moly.random() * #list) + 1
	if index > #list then
		index = #list
	end
	if #list > 1 and index == last then
		index = index % #list + 1
	end
	return index
end

-- The player and companion are further apart than `metres` on the ground.
local function apart(player, companion, metres)
	local dx, dz = player[1] - companion.position[1], player[3] - companion.position[3]
	return dx * dx + dz * dz > metres * metres
end

-- Plays one after-edit tweet row on the companion: its eye and mouth patterns,
-- its motion on the companion's own body, its emoticon (the row's, or
-- `fallback` when the row names none) and its text balloon in the steps the
-- original starts with them, then the unit's default face again. A row
-- without a motion plays the face, the balloon and the emoticon only.
-- A row's name field: the name, or nil when the row leaves it out (a JSON
-- null arrives as a value that is not a string).
local function named(value)
	if type(value) == "string" and value ~= "" then
		return value
	end
	return nil
end

local function react(row, fallback)
	-- The face and the motion first: the expression step ends at once, so the
	-- motion plays it.
	run("the tweet's face", {
		["do"] = "companion.expression",
		unit = unit,
		eye = named(row.eye),
		mouth = named(row.mouth),
	})
	-- The balloon stands beside the motion, on a task of its own.
	beside("the tweet's balloon", {
		["do"] = "companion.balloon",
		unit = unit,
		text = row.text,
		secs = BALLOON_SECS,
	})
	local emoticon = named(row.emoticon) or fallback
	local motion = named(row.motion)
	if motion ~= nil then
		local tracks = { { { ["do"] = "companion.motion", unit = unit, motion = motion } } }
		if emoticon then
			-- The emoticon starts with the motion.
			tracks[#tracks + 1] =
				{ { ["do"] = "companion.emote", unit = unit, name = emoticon, secs = EMOTE_SECS } }
		end
		run("the tweet's motion", { ["do"] = "parallel", tracks = tracks })
	elseif emoticon ~= nil then
		run("the tweet's emoticon", { ["do"] = "companion.emote", unit = unit, name = emoticon, secs = EMOTE_SECS })
	end
	run("the default face again", { ["do"] = "companion.expression", unit = unit, reset = true })
end

-- The build's ending: the companion turns to a near player, tweets a random
-- after-edit tweet of its unit with a random effect of the finale's on it when
-- the row names no emoticon.
local function finale()
	-- The build's last sounds are still ringing at its end.
	moly.wait(0.3)
	local player = player_at()
	-- The companion's entry stands while its last command is the build.
	local companion = companion_row()
	if player == nil or companion == nil then
		return
	end
	if not apart(player, companion, face_within) then
		run("companion.face the player", { ["do"] = "companion.face", unit = unit, to = "player" })
	end
	local effect = random_entry(finale_effects, picked_effect)
	picked_effect = effect
	local tweets = companion.after_edit_tweets or {}
	if #tweets == 0 then
		moly.log("unit", unit, "has no after-edit tweet")
		return
	end
	local index = random_entry(tweets, picked_tweet)
	picked_tweet = index
	local row = tweets[index]
	moly.log("after-edit tweet", row.id)
	react(row, finale_effects[effect])
end

moly.log("loaded companion demo for unit", unit)

moly.spawn(function()
	local player = wait_for_player()
	if stage and stage.setup then
		for index, step in stage.setup do
			run("setup " .. index .. " " .. tostring(step["do"]), step)
		end
		moly.log("stage set; the scene starts in", stage.start_delay or 0, "s")
	end
	if stage and stage.start_delay then
		moly.wait(stage.start_delay)
		-- The scene is placed around where the player stands once it begins.
		player = player_at() or player
	end
	local enter = if stage then offset(player, stage.enter.degrees, stage.enter.distance) else offset(player, 0, 2)

	-- The site's navigation can still be on its way when the player stands;
	-- the spawn is asked again until the site takes it.
	local spawned = false
	for _ = 1, 30 do
		spawned = run("companion.spawn", { ["do"] = "companion.spawn", unit = unit, at = enter })
		if spawned then
			break
		end
		moly.wait(1)
	end
	if not spawned then
		return
	end
	if stage then
		cue("enter")
		walk_beside("companion.walk_to its stand", stage.stand.degrees, stage.stand.distance)
	else
		-- A walk that stops short (a fixture or a character in the way) leaves
		-- the companion where it stood; the next walk starts from there.
		for round = 1, 3 do
			local degrees = if round % 2 == 1 then 180 else 0
			local name = if round % 2 == 1 then "left" else "right"
			walk_beside("round " .. round .. " to the " .. name, degrees, 2)
		end
		walk_beside("companion.walk_to its place", 90, 1)
	end

	-- The build takes an owned copy that is not placed yet, the way the layout
	-- editor's put does; the player is given one when every owned copy stands
	-- on a site.
	local fixtures = moly.observe({ "fixtures" }).fixtures
	local owned, placed = 0, 0
	for _, row in fixtures.owned do
		if row.fixture_id == build.fixture and row.texture_id == build.texture then
			owned += row.quantity
		end
	end
	for _, row in fixtures.placed do
		if row.fixture_id == build.fixture and row.texture_id == build.texture then
			placed += 1
		end
	end
	local granted = owned > placed
		or run("fixture.grant", {
			["do"] = "server.edit",
			action = "fixture.grant",
			mysekaiFixtureId = build.fixture,
			textureId = build.texture,
			quantity = placed + 1 - owned,
		})
	if granted then
		-- The owned fixtures reach the game with the server's next reply.
		moly.wait(1)
		cue("build")
		local built = run("companion.build", {
			["do"] = "companion.build",
			unit = unit,
			fixture = build.fixture,
			texture = build.texture,
			swings = build.swings,
			near = if stage and stage.near then offset(player, stage.near.degrees, stage.near.distance) else nil,
		})
		if built then
			cue("finale")
			finale()
			cue("use")
			-- The nearest placed copy of the fixture is the one just built.
			run("companion.use", { ["do"] = "companion.use", unit = unit, fixture = build.fixture })
		end
	end
	moly.log("the companion is left standing")
end)
