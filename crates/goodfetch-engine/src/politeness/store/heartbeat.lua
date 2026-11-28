-- Records an engine's heartbeat for a Politeness key, optionally registering
-- its in-flight requests as Permit leases BEFORE updating the heartbeat (on
-- reconnect), frees expired stale reservations, and computes this engine's
-- degraded share on the server's TIME (ADRs 0053, 0059 and 0142).
--
-- KEYS[1]: the key's engine heartbeats hash (`:engines`).
-- KEYS[2]: the key's Permits sorted set (`:permits`).
-- KEYS[3]: the key's Delegation (`:delegation`).
-- ARGV[1]: this engine's ID;
-- ARGV[2]: heartbeat TTL in ms (<= permit lease);
-- ARGV[3]: politeness.degraded_max in ms;
-- ARGV[4]: this engine's longest request timeout for the key in ms;
-- ARGV[5]: largest cap any run on this engine reports for the key (0 = uncapped);
-- ARGV[6]: share this engine last received and now acknowledges (0 = none yet);
-- ARGV[7]: Permit lease in ms (for reconnecting in-flight tokens);
-- ARGV[8]: count N of in-flight Permit tokens to register first (0 = normal hb);
-- ARGV[9 .. 8+N]: in-flight Permit tokens (`<engine>:<id>`).
-- Returns {granted_share, max_cap, max_timeout_ms, live_engines}, or
-- {0, 0, 0, 0} if another engine holds a live Delegation (recall first).
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)

local engine = ARGV[1]
local holder = redis.call('GET', KEYS[3])
if holder and holder ~= engine then
  return {0, 0, 0, 0}
end
local hb_ttl = tonumber(ARGV[2])
local degraded_max = tonumber(ARGV[3])
local timeout_ms = tonumber(ARGV[4])
local cap = tonumber(ARGV[5])
local acked_in = tonumber(ARGV[6])
local lease_ms = tonumber(ARGV[7])
local token_count = tonumber(ARGV[8])

redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)

if token_count > 0 then
  for i = 1, token_count do
    redis.call('ZADD', KEYS[2], now + lease_ms, ARGV[8 + i])
  end
  if redis.call('PTTL', KEYS[2]) < lease_ms then
    redis.call('PEXPIRE', KEYS[2], lease_ms)
  end
end

local function parse_engine(raw)
  local a, b, c, d, e, f, g, h = string.match(
    raw,
    '^(%d+),(%d+),(%d+),(%d+),(%d+),(%d+),(%d+),(%d+)$'
  )
  if not a then
    return nil
  end
  return tonumber(a), tonumber(b), tonumber(c), tonumber(d),
    tonumber(e), tonumber(f), tonumber(g), tonumber(h)
end

local entries = redis.call('HGETALL', KEYS[1])
local live_engines = 1
local max_cap = cap
local max_timeout = timeout_ms
local other_reserved = 0
local max_free_at = now + degraded_max + timeout_ms

for i = 1, #entries, 2 do
  local eng = entries[i]
  local raw = entries[i + 1]
  local _, stale_at, free_at, e_timeout, e_cap, e_granted, e_acked = parse_engine(raw)
  if eng ~= engine then
    if not free_at or now >= free_at then
      redis.call('HDEL', KEYS[1], eng)
    else
      if now < stale_at then
        live_engines = live_engines + 1
      end
      if e_cap > max_cap then
        max_cap = e_cap
      end
      if e_timeout > max_timeout then
        max_timeout = e_timeout
      end
      local effective = e_granted
      if e_acked > effective then
        effective = e_acked
      end
      other_reserved = other_reserved + effective
      if free_at > max_free_at then
        max_free_at = free_at
      end
    end
  end
end

local new_share = 0
if max_cap > 0 then
  local fair = math.floor(max_cap / live_engines)
  local left = max_cap - other_reserved
  if left < 0 then
    left = 0
  end
  new_share = fair
  if left < new_share then
    new_share = left
  end
  if new_share < 1 then
    new_share = 1
  end
end

local stored_acked = new_share
if new_share < acked_in then
  stored_acked = acked_in
end

local stale_at = now + hb_ttl
local free_at = now + degraded_max + timeout_ms
local encoded = string.format(
  '%d,%d,%d,%d,%d,%d,%d,%d',
  now, stale_at, free_at, timeout_ms, cap, new_share, stored_acked, degraded_max
)
redis.call('HSET', KEYS[1], engine, encoded)

local hash_ttl = (max_free_at - now) + 60000
if hash_ttl < 60000 then
  hash_ttl = 60000
end
if redis.call('PTTL', KEYS[1]) < hash_ttl then
  redis.call('PEXPIRE', KEYS[1], hash_ttl)
end

return {new_share, max_cap, max_timeout, live_engines}
