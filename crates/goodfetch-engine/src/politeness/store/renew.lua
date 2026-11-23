-- Renews a Permit's lease while its request lasts (ADR 0053), on the
-- server's time (ADR 0059). A lease that expired is lost: its room may
-- already be another holder's.
--
-- KEYS[1]: the key's Permits.
-- ARGV[1]: the holder; ARGV[2]: the lease, in milliseconds.
-- Returns 1 when renewed, 0 when the lease was lost.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local lease = tonumber(ARGV[2])
local expiry = tonumber(redis.call('ZSCORE', KEYS[1], ARGV[1]))
if expiry == nil or expiry <= now then
  redis.call('ZREM', KEYS[1], ARGV[1])
  return 0
end
redis.call('ZADD', KEYS[1], now + lease, ARGV[1])
if redis.call('PTTL', KEYS[1]) < lease then
  redis.call('PEXPIRE', KEYS[1], lease)
end
return 1
