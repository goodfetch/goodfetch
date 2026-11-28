-- Reads a Politeness key's in-flight Permit count and shared state hash,
-- or recalls another engine's live Delegation through pub/sub (ADRs 0053,
-- 0059 and 0142).
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- KEYS[2]: the key's Permits (`:permits`).
-- KEYS[3]: the key's shared state hash (`:state`).
-- ARGV[1]: this engine's ID; ARGV[2]: wakes channel.
-- Returns {2, 0, {}} when another engine holds a Delegation (recalled),
-- else {1, in_flight, HGETALL(state)}.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local holder = redis.call('GET', KEYS[1])
if holder and holder ~= ARGV[1] then
  redis.call('PUBLISH', ARGV[2], 'recall:' .. holder)
  return {2, 0, {}}
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
local in_flight = redis.call('ZCARD', KEYS[2])
local state = redis.call('HGETALL', KEYS[3])
return {1, in_flight, state}
