-- One conversation after another, staged and shot. The first talk waits for
-- the entry to settle (the launch's effect and sound must be over before the
-- shot starts); the lens then follows the speaker inside the game.
--
-- A recorder outside the game is told when to roll through the plugin's
-- store key `shoot`: { seq, talk, state } with state "roll" (the cast stands
-- and the lens frames the first speaker; the talk starts `pre_roll` seconds
-- later), "cut" (the talk has ended; the shot lasts `post_roll` seconds
-- more) or "abort" (the talk did not open).

local lens_distance = moly.config.lens_distance or 5.0
local lens_height = moly.config.lens_height or 2.6
local lens_target = moly.config.lens_target_height or 0.20
local lens_swing = moly.config.lens_swing or 0.5
local lens_fov = moly.config.lens_fov or 32
local ring = moly.config.ring or 0.7
-- The pair stands `half` metres either side of the stage's middle and the
-- player takes the third vertex `back` metres behind it, so the three bodies
-- form a triangle instead of a line. Whether the cast then look at one
-- another or at the player is the scenario's own look-at steps, not ours.
local half = moly.config.half or 0.85
local back = moly.config.back or 1.9
local settle = moly.config.settle or 8
local pre_roll = moly.config.pre_roll or 1.5
local post_roll = moly.config.post_roll or 1.0
local stage = moly.config.stage

local talks = moly.config.talks
local casts = moly.config.casts
local firsts = moly.config.firsts or {}
if talks == nil then
	talks = { moly.config.talk }
	casts = { moly.config.cast }
end

local function run(what, step)
	local outcome = moly.act(step)
	if outcome.ok then
		moly.log(what, "done", tostring(outcome.value))
	else
		moly.log(what, "FAILED:", tostring(outcome.reason))
	end
	return outcome
end

local function by_unit()
	local by = {}
	for _, c in ipairs(moly.observe({ "characters" }).characters or {}) do
		by[c.unit] = c
	end
	return by
end

local function player_at()
	local p = moly.observe({ "player" }).player
	return p and p.position
end

-- The standing roster is exactly the cast.
local function exact(cast)
	local by = by_unit()
	local count = 0
	for _ in pairs(by) do
		count = count + 1
	end
	if count ~= #cast then return false end
	for _, u in ipairs(cast) do
		if by[u] == nil then return false end
	end
	return true
end

-- Asking for a talk whose cast is not the roster makes it the roster; the
-- ask is refused while the new bodies are placed.
local function remake(talk, cast)
	run("cast", { ["do"] = "npc.play_talk", unit = cast[1], talk = talk, timeout = 20 })
	moly.wait(1)
end

-- The cast must stand and have finished arriving: bodies still in the site
-- entry are not shown yet, and a shot of them is empty.
local function arrived(cast)
	for _ = 1, 90 do
		local by = by_unit()
		local ready = true
		for _, u in ipairs(cast) do
			if by[u] == nil or by[u].objective == "EntrySite" then ready = false end
		end
		if ready then return true end
		moly.wait(1)
	end
	return false
end

local function place(cast)
	local p = player_at()
	if p == nil then return false end
	local at = stage
	if at == nil then
		at = { p[1], p[3] }
	end
	local floor = p[2]
	if #cast == 2 then
		-- A triangle: the pair either side of the stage's middle, the player on
		-- the vertex behind it, along the line the player already stood on so the
		-- picture does not swing round between talks.
		local dx, dz = p[1] - at[1], p[3] - at[2]
		local len = math.sqrt(dx * dx + dz * dz)
		if len < 1e-3 then dx, dz, len = 0.0, 1.0, 1.0 end
		local ux, uz = dx / len, dz / len
		run("player", {
			["do"] = "player.warp_to",
			to = { at[1] + ux * back, floor, at[2] + uz * back },
		})
		local sx, sz = -uz, ux
		for i = 1, 2 do
			local sign = (i == 1) and 1.0 or -1.0
			run("warp " .. tostring(cast[i]), {
				["do"] = "npc.warp_to",
				unit = cast[i],
				to = { at[1] + sx * half * sign, floor, at[2] + sz * half * sign },
				timeout = 20,
			})
		end
		return true
	end
	for i = 1, #cast do
		local angle = (i - 1) * (6.283 / #cast) + 0.6
		run("warp " .. tostring(cast[i]), {
			["do"] = "npc.warp_to",
			unit = cast[i],
			to = { at[1] + math.cos(angle) * ring, floor, at[2] + math.sin(angle) * ring },
			timeout = 20,
		})
	end
	return true
end

local function stand(talk, cast)
	for _ = 1, 8 do
		if not exact(cast) then
			remake(talk, cast)
		end
		if arrived(cast) and exact(cast) then
			if not place(cast) then return false end
			moly.wait(0.3)
			return true
		end
	end
	return false
end

local seq = 0
local function signal(state, talk)
	moly.store.set("shoot", { seq = seq, talk = talk, state = state })
end

local function lens(first)
	run("lens", {
		["do"] = "camera.talk_lens",
		distance = lens_distance,
		height = lens_height,
		target = lens_target,
		fov = lens_fov,
		swing = lens_swing,
		unit = first,
	})
end

local function open(talk, cast)
	for _ = 1, 6 do
		if run("play", { ["do"] = "npc.play_talk", unit = cast[1], talk = talk, timeout = 20 }).ok
			and moly.act({ ["do"] = "wait_until", ["until"] = "talk_active", timeout = 10 }).ok then
			return true
		end
		moly.wait(2)
	end
	return false
end

moly.spawn(function()
	while player_at() == nil do
		moly.wait(1)
	end
	-- The launch's own entry effect and sound must be over.
	moly.wait(settle)

	for index = 1, #talks do
		local talk = talks[index]
		local cast = casts[index]
		seq = seq + 1
		moly.log("talk", tostring(talk), "cast", tostring(#cast))
		if not stand(talk, cast) then
			moly.log("talk", tostring(talk), "never stood")
			signal("abort", talk)
		else
			lens(firsts[index] or cast[1])
			signal("roll", talk)
			moly.wait(pre_roll)
			if not open(talk, cast) then
				moly.log("talk", tostring(talk), "never opened")
				signal("abort", talk)
			else
				while true do
					local state = moly.observe({ "talk" }).talk
					if state and state.active == false then break end
					moly.wait(0.1)
				end
				signal("cut", talk)
				moly.wait(post_roll + 0.5)
			end
		end
	end
	seq = seq + 1
	signal("end", 0)
	moly.log("all", "done")
end)
