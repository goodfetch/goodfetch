-- Releases this engine's Delegation of a Politeness key, writes any
-- in-flight Permits as leased entries and saves shared state, then
-- publishes 'recalled' on the key's wakes channel (ADRs 0053, 0059, 0142).
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- KEYS[2]: the key's Permits (`:permits`).
-- KEYS[3]: the key's shared state hash (`:state`).
-- ARGV[1]: this engine's ID; ARGV[2]: Permit lease in ms;
-- ARGV[3]: wakes channel; ARGV[4]: state TTL in ms;
-- ARGV[5]: permit count N;
-- ARGV[6 .. 5+N]: in-flight Permit tokens;
-- ARGV[6+N ..]: alternating state part names and JSON values.
-- Returns 1 if this engine held the Delegation, else 0.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local engine = ARGV[1]
local lease = tonumber(ARGV[2])
local channel = ARGV[3]
local state_ttl = tonumber(ARGV[4])
local permit_count = tonumber(ARGV[5])
local held = redis.call('GET', KEYS[1]) == engine
if held then
  redis.call('DEL', KEYS[1])
end
if permit_count > 0 then
  for i = 1, permit_count do
    redis.call('ZADD', KEYS[2], now + lease, ARGV[5 + i])
  end
  if redis.call('PTTL', KEYS[2]) < lease then
    redis.call('PEXPIRE', KEYS[2], lease)
  end
end
local state_start = 6 + permit_count
if state_start <= #ARGV then
  for i = state_start, #ARGV, 2 do
    redis.call('HSET', KEYS[3], ARGV[i], ARGV[i + 1])
  end
  if redis.call('PTTL', KEYS[3]) < state_ttl then
    redis.call('PEXPIRE', KEYS[3], state_ttl)
  end
end
if held then
  redis.call('PUBLISH', channel, 'recalled')
  return 1
end
return 0
