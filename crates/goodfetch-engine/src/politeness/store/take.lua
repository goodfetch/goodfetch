-- Takes a Permit of a key as a lease (ADR 0053), or refuses it when the key
-- is full. The time is the server's (ADR 0059), so engines whose clocks
-- drift agree on every lease.
--
-- KEYS[1]: the key's Permits, a sorted set of holders scored by the
--          millisecond their lease expires.
-- ARGV[1]: the holder; ARGV[2]: the key's cap; ARGV[3]: the lease, in
--          milliseconds.
-- Returns {1, in_flight} when granted, {0, in_flight} when full.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local lease = tonumber(ARGV[3])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
local in_flight = redis.call('ZCARD', KEYS[1])
if in_flight >= tonumber(ARGV[2]) then
  return {0, in_flight}
end
redis.call('ZADD', KEYS[1], now + lease, ARGV[1])
if redis.call('PTTL', KEYS[1]) < lease then
  redis.call('PEXPIRE', KEYS[1], lease)
end
return {1, in_flight + 1}
