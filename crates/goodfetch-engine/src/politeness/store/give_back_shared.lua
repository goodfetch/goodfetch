-- Gives a shared Permit back, persists any updated shared state, wakes
-- waiters through pub/sub, and reports whether any other engine still
-- holds an unexpired Permit of the key (ADRs 0053, 0059 and 0142).
--
-- KEYS[1]: `:permits`; KEYS[2]: `:state`.
-- ARGV[1]: this engine's token prefix (`<engine>:` ); ARGV[2]: Permit token;
-- ARGV[3]: wakes channel; ARGV[4]: state TTL ms;
-- ARGV[5]: state part name ('' = none); ARGV[6]: state JSON value.
-- Returns {given, alone} where alone=1 if no other engine holds a Permit.
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
return {given, alone}
