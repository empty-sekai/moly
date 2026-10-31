-- Home build: each configured builder is a companion with a queue of
-- fixtures. A builder puts its fixtures on the site one after another; each
-- stands as a ghost and the companion hammers it solid (companion.build).
-- A build ends with no reaction, so the next one starts at once. Builders work
-- side by side. Once every queue is done the builders react together, once:
-- each turns to `face`, tweets one of its unit's after-edit tweets and plays
-- an effect, then uses the fixture named by its `use`. A builder with a
-- `leave` point then walks there, all of them together.
--
-- The fixtures a queue needs are granted up front, so a builder never waits
-- on a grant between two builds.
--
-- A recorder outside the game is told the phase through the plugin's store
-- key `shoot`: { state, built, total } with state "enter" (the builders walk
-- in), "build" (a build ended; `built` of `total` stand), "react" (every
-- queue is done and the builders react), "leave" (the builders walk to their
-- `leave` points) or "end" (the scene is over).
--
-- Optional `stage` keys: `setup` (director steps run in order once the player
-- stands, such as placing the player and the camera), `start_delay` (seconds
-- before the scene) and `cues` (director steps by beat, "build", "react" or
-- "use", each started once beside the beat's first moment).

local builders = moly.config.builders or {}
local put_gap = moly.config.put_gap or 2.5
local default_swings = moly.config.swings
local default_direction = moly.config.direction
local celebrate = moly.config.celebrate ~= false
local face = moly.config.face or "player"
local face_within = moly.config.face_player_within or 6
local stagger = moly.config.finale_stagger or 0.4
local finale_effects = moly.config.finale_effects or { "fx_emote_018_single", "fx_emote_010" }
local stage = moly.config.stage
-- How long a tweet balloon stays up, seconds, and how long an emoticon plays.
local BALLOON_SECS = 5
local EMOTE_SECS = 2

local player = nil
-- The earliest start of the next build; builds claim consecutive slots.
local next_slot = 0
local built, total = 0, 0
local fired = {}
local picked_tweet, picked_effect = {}, {}

-- A point `distance` metres from `from` along a direction `degrees` from +x.
local function offset(from, degrees, distance)
	local radians = degrees * math.pi / 180
	return { from[1] + math.cos(radians) * distance, from[2], from[3] + math.sin(radians) * distance }
end

-- A configured point: a world point [x, y, z] as given, or { degrees,
-- distance } from the player.
local function point(spec)
	if spec[1] ~= nil then
		return { spec[1], spec[2], spec[3] }
	end
	return offset(player, spec.degrees, spec.distance)
end

-- Runs one step and logs its outcome.
local function run(what, step)
	local outcome = moly.act(step)
	if outcome.ok then
		if type(outcome.value) == "string" and outcome.value ~= "" then
			moly.log(what, "done:", outcome.value)
		else
			moly.log(what, "done")
		end
	else
		moly.log(what, "failed:", tostring(outcome.reason))
	end
	return outcome.ok
end

-- Runs one step on a task of its own, beside the task that started it.
local function beside(what, step)
	moly.spawn(function()
		run(what, step)
	end)
end

local function signal(state)
	moly.store.set("shoot", { state = state, built = built, total = total })
end

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

-- Starts the stage's cues of `beat` on a task of their own, once.
local function cue(beat)
	local steps = stage and stage.cues and stage.cues[beat]
	if steps == nil or fired[beat] then
		return
	end
	fired[beat] = true
	moly.spawn(function()
		for index, step in steps do
			run("cue " .. beat .. " " .. index .. " " .. tostring(step["do"]), step)
		end
	end)
end

local function companion_row(unit)
	for _, row in moly.observe({ "companions" }).companions do
		if row.unit == unit then
			return row
		end
	end
	return nil
end

-- One random index of `list`, not the one the last pick gave.
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

local function apart(from, companion, metres)
	local dx, dz = from[1] - companion.position[1], from[3] - companion.position[3]
	return dx * dx + dz * dz > metres * metres
end

-- A row's name field: the name, or nil when the row leaves it out.
local function named(value)
	if type(value) == "string" and value ~= "" then
		return value
	end
	return nil
end

-- Plays one after-edit tweet row on the companion: its face, its motion, its
-- emoticon (the row's, or `fallback` when the row names none) and its text
-- balloon, then the unit's default face again.
local function react(unit, row, fallback)
	run("the tweet's face", {
		["do"] = "companion.expression",
		unit = unit,
		eye = named(row.eye),
		mouth = named(row.mouth),
	})
	beside("the tweet's balloon", { ["do"] = "companion.balloon", unit = unit, text = row.text, secs = BALLOON_SECS })
	local emoticon = named(row.emoticon) or fallback
	local motion = named(row.motion)
	if motion ~= nil then
		local tracks = { { { ["do"] = "companion.motion", unit = unit, motion = motion } } }
		if emoticon then
			tracks[#tracks + 1] = { { ["do"] = "companion.emote", unit = unit, name = emoticon, secs = EMOTE_SECS } }
		end
		run("the tweet's motion", { ["do"] = "parallel", tracks = tracks })
	elseif emoticon ~= nil then
		run("the tweet's emoticon", { ["do"] = "companion.emote", unit = unit, name = emoticon, secs = EMOTE_SECS })
	end
	run("the default face again", { ["do"] = "companion.expression", unit = unit, reset = true })
end

-- One builder's reaction: turn, then a random after-edit tweet of its unit
-- with a random finale effect on it when the row names no emoticon.
local function reaction(unit)
	local companion = companion_row(unit)
	if companion == nil then
		return
	end
	if face == "player" then
		local at = player_at()
		if at ~= nil and not apart(at, companion, face_within) then
			run("companion.face the player", { ["do"] = "companion.face", unit = unit, to = "player" })
		end
	else
		run("companion.face the mark", { ["do"] = "companion.face", unit = unit, to = face })
	end
	local effect = random_entry(finale_effects, picked_effect[unit])
	picked_effect[unit] = effect
	local tweets = companion.after_edit_tweets or {}
	if #tweets == 0 then
		moly.log("unit", unit, "has no after-edit tweet")
		return
	end
	local index = random_entry(tweets, picked_tweet[unit])
	picked_tweet[unit] = index
	moly.log("after-edit tweet", unit, tweets[index].id)
	react(unit, tweets[index], finale_effects[effect])
end

-- The fixtures the queues need, granted where the player owns too few spare
-- copies. Owned counts every copy, placed or not.
local function grant_queues()
	local needs = {}
	for _, builder in builders do
		for _, item in builder.queue do
			local key = tostring(item.fixture) .. ":" .. tostring(item.texture or 1)
			local need = needs[key]
			if need == nil then
				needs[key] = { fixture = item.fixture, texture = item.texture or 1, count = 1 }
			else
				need.count += 1
			end
		end
	end
	local fixtures = moly.observe({ "fixtures" }).fixtures
	local granted = false
	for _, need in needs do
		local owned, placed = 0, 0
		for _, row in fixtures.owned do
			if row.fixture_id == need.fixture and row.texture_id == need.texture then
				owned += row.quantity
			end
		end
		for _, row in fixtures.placed do
			if row.fixture_id == need.fixture and row.texture_id == need.texture then
				placed += 1
			end
		end
		local short = placed + need.count - owned
		if short > 0 then
			if not run("fixture.grant " .. need.fixture, {
				["do"] = "server.edit",
				action = "fixture.grant",
				mysekaiFixtureId = need.fixture,
				textureId = need.texture,
				quantity = short,
			}) then
				return false
			end
			granted = true
		end
	end
	if granted then
		-- The owned fixtures reach the game with the server's next reply.
		moly.wait(1)
	end
	return true
end

-- One builder: enter, then build the queue in order.
local function work(builder)
	local unit = builder.unit
	local enter = builder.enter or { degrees = 180, distance = 3 }
	local spawned = false
	-- The site's navigation can still be on its way when the player stands;
	-- the spawn is asked again until the site takes it.
	local tries = (stage and stage.spawn_tries) or 30
	for _ = 1, tries do
		spawned = run("companion.spawn " .. unit, {
			["do"] = "companion.spawn",
			unit = unit,
			at = point(enter),
		})
		if spawned then
			break
		end
		moly.wait(1)
	end
	if not spawned then
		return
	end
	-- Lets the camera settle on the site before the first build starts.
	if stage and stage.build_delay then
		moly.wait(stage.build_delay)
	end
	for _, item in builder.queue do
		-- Builds claim consecutive slots, so no two start within `put_gap`
		-- and the fixtures land one after another.
		local slot = math.max(moly.now(), next_slot)
		next_slot = slot + put_gap
		local lag = slot - moly.now()
		if lag > 0 then
			moly.wait(lag)
		end
		cue("build")
		-- `at` is the exact cell; `near` only starts the put search.
		local at, near = nil, nil
		if item.at then
			at = point(item.at)
		elseif item.near then
			near = point(item.near)
		end
		local direction = item.direction
		if direction == nil then
			direction = builder.direction or default_direction
		end
		local ok = run("companion.build " .. unit .. " fixture " .. item.fixture, {
			["do"] = "companion.build",
			unit = unit,
			fixture = item.fixture,
			texture = item.texture or 1,
			swings = item.swings or default_swings,
			at = at,
			near = near,
			direction = direction,
		})
		if ok then
			built += 1
			signal("build")
		end
	end
end

moly.log("loaded home build for", #builders, "builders")

moly.spawn(function()
	player = wait_for_player()
	if stage and stage.setup then
		for index, step in stage.setup do
			run("setup " .. index .. " " .. tostring(step["do"]), step)
		end
	end
	if stage and stage.start_delay then
		moly.wait(stage.start_delay)
		-- The scene is placed around where the player stands once it begins.
		player = player_at() or player
	end
	for _, builder in builders do
		total += #builder.queue
	end
	if not grant_queues() then
		moly.log("the fixtures were not granted; the scene is left")
		return
	end
	signal("enter")

	local working = #builders
	for _, builder in builders do
		moly.spawn(function()
			work(builder)
			working -= 1
		end)
	end
	while working > 0 do
		moly.wait(0.5)
	end

	if celebrate then
		-- The last build's sounds are still ringing at its end.
		moly.wait(0.3)
		signal("react")
		cue("react")
		local reacting = #builders
		for index, builder in builders do
			moly.spawn(function()
				moly.wait((index - 1) * stagger)
				reaction(builder.unit)
				if builder.use then
					cue("use")
					-- The nearest placed copy of the fixture to the companion.
					run("companion.use " .. builder.unit, {
						["do"] = "companion.use",
						unit = builder.unit,
						fixture = builder.use,
					})
				end
				reacting -= 1
			end)
		end
		while reacting > 0 do
			moly.wait(0.5)
		end
	end

	local leaving = 0
	for _, builder in builders do
		if builder.leave then
			leaving += 1
		end
	end
	if leaving > 0 then
		signal("leave")
		moly.log("the builders leave for the next site")
		for _, builder in builders do
			if builder.leave then
				moly.spawn(function()
					run("companion.walk_to " .. builder.unit, {
						["do"] = "companion.walk_to",
						unit = builder.unit,
						to = point(builder.leave),
					})
					leaving -= 1
				end)
			end
		end
		while leaving > 0 do
			moly.wait(0.5)
		end
	end
	signal("end")
	moly.log("the builders are left standing")
end)
