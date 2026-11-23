-- Gives a Permit of a key back and wakes the key's waiters through pub/sub
-- (ADR 0053). A lease that already expired gives nothing back and wakes
-- no one: its room was freed when it expired.
--
-- KEYS[1]: the key's Permits.
-- ARGV[1]: the holder; ARGV[2]: the key's wake channel.
-- Returns 1 when the Permit was held, else 0.
local given = redis.call('ZREM', KEYS[1], ARGV[1])
if given == 1 then
  redis.call('PUBLISH', ARGV[2], ARGV[1])
end
return given
