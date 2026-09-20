local M = {}

function M.shoot(api)
    api:safeAsyncTask(function ()
        api:shootOnce(api:isShootingNeedConsumeAmmo())
        return false
    end,1*1000,0,1)
end

function M.start_reload(api)
    local cache = {
        reloaded_flag = 0,
        max_ammo = 6,
        needed_count = api:getNeededAmmoAmount()
    }
    api:cacheScriptData(cache)
end

function M.tick_reload(api)
    local cache = api:getCachedScriptData()
    local reload_feed = 2 * 1000
    local reload_cooldown = 3.125 * 1000
    local need_ammo = api:getNeededAmmoAmount()
    local reload_time = api:getReloadTime()
    --reload
    if (cache.reloaded_flag == 0) then
        if(reload_time >= reload_feed and reload_time < reload_cooldown) then
            api:putAmmoInMagazine(api:consumeAmmoFromPlayer(cache.max_ammo))
            cache.reloaded_flag = 1
            return TACTICAL_RELOAD_FINISHING, reload_cooldown - reload_time
        elseif(reload_time >= reload_cooldown) then
            return NOT_RELOADING, -1
        elseif(reload_time < reload_feed) then
            return TACTICAL_RELOAD_FEEDING, reload_feed - reload_time
        end
    elseif(cache.reloaded_flag == 1)then
        if(reload_time >= reload_cooldown)then
            return NOT_RELOADING, -1
        else 
            return TACTICAL_RELOAD_FINISHING, reload_cooldown - reload_time
        end
    end
    -- regular return
    return NOT_RELOADING, -1
end

return M