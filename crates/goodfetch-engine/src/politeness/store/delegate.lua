-- Acquires a Delegation of a Politeness key in the store (ADR 0142), on
-- the server's time (ADR 0059). Granted only when no other engine holds
-- the Delegation, no unexpired Permit is in flight, and no other engine
-- holds a stale-reserved heartbeat record (ADR 0053). Optionally registers
-- this engine's initial heartbeat atomically in the same round trip so
-- acquiring a Delegation costs exactly one EVALSHA.
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- KEYS[2]: the key's Permits (`:permits`).
-- KEYS[3]: the key's shared state hash (`:state`).
-- KEYS[4]: the key's engine heartbeats hash (`:engines`).
-- ARGV[1]: this engine's ID; ARGV[2]: Delegation TTL in milliseconds;
-- Optional (all 5 passed together when registering initial heartbeat):
-- ARGV[3]: heartbeat TTL ms (0 = skip heartbeat registration);
-- ARGV[4]: politeness.degraded_max ms;
-- ARGV[5]: longest request timeout ms;
-- ARGV[6]: cap (0 = uncapped);
-- ARGV[7]: wakes channel (to notify live idle engines that Delegation was taken).
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
local entries = redis.call('HGETALL', KEYS[4])
local stale_other = false
local live_others = {}
for i = 1, #entries, 2 do
  local eng = entries[i]
  local raw = entries[i + 1]
  if eng ~= engine then
    local _, stale_at, free_at = string.match(raw, '^(%d+),(%d+),(%d+),')
    stale_at = tonumber(stale_at)
    free_at = tonumber(free_at)
    if not free_at or now >= free_at then
      redis.call('HDEL', KEYS[4], eng)
    elseif not stale_at or now >= stale_at then
      -- Another engine is stale: its reservation must hold until free_at.
      stale_other = true
    else
      live_others[#live_others + 1] = eng
    end
  end
end
if stale_other then
  return {0, {''}}
end
for i = 1, #live_others do
  redis.call('HDEL', KEYS[4], live_others[i])
end
redis.call('SET', KEYS[1], engine, 'PX', ttl)
local hb_ttl = tonumber(ARGV[3] or '0') or 0
if hb_ttl > 0 then
  local degraded_max = tonumber(ARGV[4])
  local timeout_ms = tonumber(ARGV[5])
  local cap = tonumber(ARGV[6])
  local stale_at = now + hb_ttl
  local free_at = now + degraded_max + timeout_ms
  local encoded = string.format(
    '%d,%d,%d,%d,%d,%d,%d,%d',
    now, stale_at, free_at, timeout_ms, cap, cap, cap, degraded_max
  )
  redis.call('HSET', KEYS[4], engine, encoded)
  local hash_ttl = degraded_max + timeout_ms + 60000
  if hash_ttl < 60000 then
    hash_ttl = 60000
  end
  if redis.call('PTTL', KEYS[4]) < hash_ttl then
    redis.call('PEXPIRE', KEYS[4], hash_ttl)
  end
end
if #live_others > 0 and ARGV[7] and ARGV[7] ~= '' then
  redis.call('PUBLISH', ARGV[7], 'delegated:' .. engine)
end
local state = redis.call('HGETALL', KEYS[3])
return {1, state}
