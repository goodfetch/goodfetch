-- Gives a shared Permit back, persists any updated shared state, wakes
-- waiters through pub/sub, and reports whether any other engine still
-- holds an unexpired Permit or stale reservation of the key
-- (ADRs 0053, 0059 and 0142).
--
-- KEYS[1]: `:permits`; KEYS[2]: `:state`; KEYS[3]: `:engines`.
-- ARGV[1]: this engine's token prefix (`<engine>:`); ARGV[2]: Permit token;
-- ARGV[3]: wakes channel; ARGV[4]: state TTL ms;
-- ARGV[5]: state part name ('' = none); ARGV[6]: state JSON value;
-- ARGV[7]: this engine's ID.
-- Returns {given, alone} where alone=1 if no other engine holds a Permit or stale reservation.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local given = redis.call('ZREM', KEYS[1], ARGV[2])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
if ARGV[5] ~= '' then
  redis.call('HSET', KEYS[2], ARGV[5], ARGV[6])
  local state_ttl = tonumber(ARGV[4])
  if redis.call('PTTL', KEYS[2]) < state_ttl then
    redis.call('PEXPIRE', KEYS[2], state_ttl)
  end
end
if given == 1 then
  redis.call('PUBLISH', ARGV[3], 'wake')
end
local prefix = ARGV[1]
local prefix_len = #prefix
local members = redis.call('ZRANGE', KEYS[1], 0, -1)
local alone = 1
for i = 1, #members do
  if string.sub(members[i], 1, prefix_len) ~= prefix then
    alone = 0
    break
  end
end
if alone == 1 then
  local entries = redis.call('HGETALL', KEYS[3])
  for i = 1, #entries, 2 do
    local eng = entries[i]
    local raw = entries[i + 1]
    if eng ~= ARGV[7] then
      local _, stale_at, free_at = string.match(raw, '^(%d+),(%d+),(%d+),')
      stale_at = tonumber(stale_at)
      free_at = tonumber(free_at)
      if not free_at or now >= free_at then
        redis.call('HDEL', KEYS[3], eng)
      elseif not stale_at or now >= stale_at then
        alone = 0
        break
      end
    end
  end
end
return {given, alone}
