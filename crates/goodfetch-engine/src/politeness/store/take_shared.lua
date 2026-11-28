-- Takes a shared Permit of a Politeness key as a lease, enforcing stale
-- engines' reserved shares, optional GCRA rate pacing, and shared state
-- updates on server TIME (ADRs 0053, 0059 and 0142). Recalls another
-- engine's live Delegation through pub/sub.
--
-- KEYS[1]: `:delegation`; KEYS[2]: `:permits`; KEYS[3]: `:pace`;
-- KEYS[4]: `:state`; KEYS[5]: `:engines`.
-- ARGV[1]: engine ID; ARGV[2]: Permit token; ARGV[3]: cap (0 = uncapped);
-- ARGV[4]: lease ms; ARGV[5]: wakes channel; ARGV[6]: GCRA interval ms (0 = none);
-- ARGV[7]: GCRA burst; ARGV[8]: state TTL ms;
-- ARGV[9]: state part name ('' = none); ARGV[10]: state JSON value.
-- Returns {0, effective_in_flight} Full, {1, effective_in_flight} Granted,
-- {2, 0} Recalled, or {3, wait_ms} RateWait.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local holder = redis.call('GET', KEYS[1])
if holder and holder ~= ARGV[1] then
  redis.call('PUBLISH', ARGV[5], 'recall:' .. holder)
  return {2, 0}
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
local members = redis.call('ZRANGE', KEYS[2], 0, -1)
local in_flight = #members

local stale_extra = 0
local entries = redis.call('HGETALL', KEYS[5])
for i = 1, #entries, 2 do
  local eng = entries[i]
  local raw = entries[i + 1]
  local _, stale_at, free_at, _, _, granted, acked = string.match(
    raw,
    '^(%d+),(%d+),(%d+),(%d+),(%d+),(%d+),(%d+),'
  )
  stale_at = tonumber(stale_at)
  free_at = tonumber(free_at)
  granted = tonumber(granted) or 0
  acked = tonumber(acked) or 0
  if not free_at or now >= free_at then
    redis.call('HDEL', KEYS[5], eng)
  elseif eng ~= ARGV[1] and now >= stale_at then
    local reserved = granted
    if acked > reserved then
      reserved = acked
    end
    local prefix = eng .. ':'
    local prefix_len = #prefix
    local held = 0
    for j = 1, #members do
      if string.sub(members[j], 1, prefix_len) == prefix then
        held = held + 1
      end
    end
    if reserved > held then
      stale_extra = stale_extra + (reserved - held)
    end
  end
end

local effective = in_flight + stale_extra
local cap = tonumber(ARGV[3])
if cap > 0 and effective >= cap then
  return {0, effective}
end
local interval = tonumber(ARGV[6])
if interval > 0 then
  local tolerance = interval * tonumber(ARGV[7])
  local arrival = tonumber(redis.call('GET', KEYS[3])) or now
  if arrival < now then
    arrival = now
  end
  local admitted_at = arrival - tolerance
  if now < admitted_at then
    return {3, admitted_at - now}
  end
  local next_arrival = arrival + interval
  redis.call('SET', KEYS[3], next_arrival, 'PX', next_arrival - now)
end
local lease = tonumber(ARGV[4])
redis.call('ZADD', KEYS[2], now + lease, ARGV[2])
if redis.call('PTTL', KEYS[2]) < lease then
  redis.call('PEXPIRE', KEYS[2], lease)
end
if ARGV[9] ~= '' then
  redis.call('HSET', KEYS[4], ARGV[9], ARGV[10])
  local state_ttl = tonumber(ARGV[8])
  if redis.call('PTTL', KEYS[4]) < state_ttl then
    redis.call('PEXPIRE', KEYS[4], state_ttl)
  end
end
return {1, effective + 1}
