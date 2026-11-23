-- A key's rate as GCRA (ADR 0053; redis-cell's `CL.THROTTLE` is the
-- reference), on the server's time (ADR 0059): a start is admitted once
-- the key's theoretical arrival time, less the burst's tolerance, has
-- come, and each admitted start moves that time on by the interval.
--
-- KEYS[1]: the key's theoretical arrival time, in milliseconds.
-- ARGV[1]: the interval between starts, in milliseconds; ARGV[2]: the
--          starts admitted at once beyond the first.
-- Returns {1, 0} when admitted, {0, wait} with the milliseconds left
-- before a start is.
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local interval = tonumber(ARGV[1])
local tolerance = interval * tonumber(ARGV[2])
local arrival = tonumber(redis.call('GET', KEYS[1])) or now
if arrival < now then
  arrival = now
end
local admitted_at = arrival - tolerance
if now < admitted_at then
  return {0, admitted_at - now}
end
local next_arrival = arrival + interval
redis.call('SET', KEYS[1], next_arrival, 'PX', next_arrival - now)
return {1, 0}
