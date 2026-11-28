-- Removes an engine's heartbeat record for a Politeness key on clean exit
-- (ADR 0053).
--
-- KEYS[1]: the key's engine heartbeats hash (`:engines`).
-- ARGV[1]: this engine's ID.
-- Returns 1 if removed, else 0.
return redis.call('HDEL', KEYS[1], ARGV[1])
