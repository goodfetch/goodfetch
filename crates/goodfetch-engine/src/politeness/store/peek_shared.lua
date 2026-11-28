-- Reads a Politeness key's effective in-flight Permit count (including stale
-- engines' reserved shares, ADR 0053) and shared state hash, or recalls
-- another engine's live Delegation through pub/sub (ADRs 0053, 0059, 0142).
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- KEYS[2]: the key's Permits (`:permits`).
-- KEYS[3]: the key's shared state hash (`:state`).
-- KEYS[4]: the key's engine heartbeats hash (`:engines`).
-- ARGV[1]: this engine's ID; ARGV[2]: wakes channel.
-- Returns {2, 0, {}} when another engine holds a Delegation (recalled),
-- else {1, effective_in_flight, HGETALL(state)}.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local holder = redis.call('GET', KEYS[1])
if holder and holder ~= ARGV[1] then
  redis.call('PUBLISH', ARGV[2], 'recall:' .. holder)
  return {2, 0, {}}
end
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', now)
local members = redis.call('ZRANGE', KEYS[2], 0, -1)
local in_flight = #members

local stale_extra = 0
local entries = redis.call('HGETALL', KEYS[4])
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
    redis.call('HDEL', KEYS[4], eng)
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

local state = redis.call('HGETALL', KEYS[3])
return {1, in_flight + stale_extra, state}
