-- Renews this engine's Delegation of a Politeness key on the server's
-- clock (ADRs 0059 and 0142). Returns 1 when still held and renewed, 0
-- when expired or held by another engine.
--
-- KEYS[1]: the key's Delegation (`:delegation`).
-- ARGV[1]: this engine's ID; ARGV[2]: Delegation TTL in milliseconds.
if redis.call('GET', KEYS[1]) ~= ARGV[1] then
  return 0
end
redis.call('PEXPIRE', KEYS[1], tonumber(ARGV[2]))
return 1
