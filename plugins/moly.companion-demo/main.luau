-- The companion demo: it reads the player's position with moly.observe, spawns
-- the companion of the configured unit two metres beside the player, waits
-- until the spawn is done, walks the companion to either side of the player
-- three times, then leaves it standing a metre from the player.
--
-- Each step goes through moly.act, which runs a director step and resumes the
-- task with `{ ok = true }` or `{ ok = false, reason = ... }`.

local unit = moly.config.unit or 14

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

-- Walks the companion `distance` metres from the player along `degrees`.
local function walk_beside(what, degrees, distance)
	local player = player_at() or wait_for_player()
	return run(what, { ["do"] = "companion.walk_to", unit = unit, to = offset(player, degrees, distance) })
end

moly.log("loaded companion demo for unit", unit)

moly.spawn(function()
	local player = wait_for_player()

	-- The site's navigation can still be on its way when the player stands;
	-- the spawn is asked again until the site takes it.
	local spawned = false
	for _ = 1, 30 do
		spawned = run("companion.spawn", { ["do"] = "companion.spawn", unit = unit, at = offset(player, 0, 2) })
		if spawned then
			break
		end
		moly.wait(1)
	end
	if not spawned then
		return
	end
	-- A walk that stops short (a fixture or a character in the way) leaves the
	-- companion where it stood; the next walk starts from there.
	for round = 1, 3 do
		local degrees = if round % 2 == 1 then 180 else 0
		local name = if round % 2 == 1 then "left" else "right"
		walk_beside("round " .. round .. " to the " .. name, degrees, 2)
	end
	walk_beside("companion.walk_to its place", 90, 1)
	moly.log("the companion is left standing")
end)
