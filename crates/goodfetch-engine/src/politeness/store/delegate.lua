-- Acquires a Delegation of a Politeness key in the store (ADR 0142), on
-- the server's time (ADR 0059). Granted only when no other engine holds
-- the Delegation and no unexpired Permit is in flight.
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- KEYS[2]: the key's Permits (`:permits`).
-- KEYS[3]: the key's shared state hash (`:state`).
-- ARGV[1]: this engine's ID; ARGV[2]: Delegation TTL in milliseconds.
-- Returns {1, HGETALL(state)} when granted, {0, {holder_or_empty}} when not.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local engine = ARGV[1]
local ttl = tonumber(ARGV[2])
local holder = redis.call('GET', KEYS[1])
if holder and holder ~= engine then
  return {0, {holder}}
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
if redis.call('ZCARD', KEYS[2]) > 0 then
  return {0, {''}}
end
redis.call('SET', KEYS[1], engine, 'PX', ttl)
local state = redis.call('HGETALL', KEYS[3])
return {1, state}
