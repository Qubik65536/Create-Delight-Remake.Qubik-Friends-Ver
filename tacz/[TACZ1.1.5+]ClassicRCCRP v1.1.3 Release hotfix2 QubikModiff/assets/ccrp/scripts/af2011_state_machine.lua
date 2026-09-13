local default = require("tacz_default_state_machine")
local GUN_KICK_TRACK_LINE = default.GUN_KICK_TRACK_LINE

local STATIC_TRACK_LINE = default.STATIC_TRACK_LINE
local MAIN_TRACK = default.MAIN_TRACK
local BOLT_CAUGHT_TRACK = default.BOLT_CAUGHT_TRACK
local bolt_caught_states = default.bolt_caught_states
local main_track_states = default.main_track_states

local gun_kick_state = setmetatable({}, {__index = default.gun_kick_state})
local idle_state = setmetatable({}, {__index = main_track_states.idle})

local function get_ejection_time(context)
    local ejection_time = context:getStateMachineParams().shell_ejecting_time
    if (ejection_time) then
        ejection_time = ejection_time * 1000
    else
        ejection_time = 0
    end
    return ejection_time
end

local function isNoAmmo(context)
    -- 这里同时检查了枪管和弹匣
    return (not context:hasBulletInBarrel()) and (context:getAmmoCount() <= 0)
end

function idle_state.transition(this, context, input)
    if (input == INPUT_SHOOT) then
        idle_state.timestamp = context:getCurrentTimestamp()
        idle_state.ejection_time = 50
    end
    return main_track_states.idle.transition(this, context, input)
end

function idle_state.update(this, context)
    if (idle_state.timestamp ~= nil) then
        if (idle_state.timestamp ~= -1 and context:getCurrentTimestamp() - idle_state.timestamp > idle_state.ejection_time) then
            context:popShellFrom(0)
            idle_state.timestamp = -1
        end
    end
end

local M = setmetatable({
    main_track_states = setmetatable({
        idle = idle_state
    }, {__index = main_track_states})
}, {__index = default})

function M:initialize(context)
    default.initialize(self, context)
end

return M