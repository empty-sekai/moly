-- The example plugin: it greets on load, waits two seconds, runs a director
-- step that logs a line, and logs what the step answered. The step goes
-- through moly.act, which runs the director's own `log` action and resumes
-- the task with `{ ok = true, value = <the line> }`.

moly.log("loaded", tostring(moly.config.greeting))

moly.spawn(function()
	moly.wait(2)
	local outcome = moly.act({ ["do"] = "log", text = "the example plugin ran a step" })
	if outcome.ok then
		moly.log("the step answered", tostring(outcome.value))
	else
		moly.log("the step was refused:", tostring(outcome.reason))
	end
end)
