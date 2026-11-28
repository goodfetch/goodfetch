-- Saves one part of a Politeness key's shared state hash in the store
-- (ADRs 0053 and 0142), e.g. when a site asks for a backoff, Retry-After,
-- Crawl-delay or provider refusal ceiling.
--
-- KEYS[1]: `:state`.
-- ARGV[1]: state TTL ms; ARGV[2]: state part name; ARGV[3]: state JSON value.
redis.call('HSET', KEYS[1], ARGV[2], ARGV[3])
local state_ttl = tonumber(ARGV[1])
if redis.call('PTTL', KEYS[1]) < state_ttl then
  redis.call('PEXPIRE', KEYS[1], state_ttl)
end
return 1
