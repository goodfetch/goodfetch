PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE entries (
        sequence INTEGER NOT NULL PRIMARY KEY,
        entry TEXT NOT NULL,
        hash TEXT NOT NULL
    ) STRICT;
INSERT INTO "entries" VALUES(1,'{"actor":{"kind":"engine"},"details":{"setting":"security.browser_sandbox"},"kind":"security_switch_turned_off","layer":"operator_policy","run_id":"weakened","sequence":1,"time":"2026-01-05T09:00:00Z"}','9826e7b1d1bab951d623ee4c18af4aceea74cdb1c51e440a6a7b8b7ae95b7823');
INSERT INTO "entries" VALUES(2,'{"actor":{"kind":"client_surface","surface":"cli"},"details":{"domain":"example.com","placeholder":"%password%","reference":"env:SITE_PASSWORD"},"kind":"secret_reference_used","layer":"run","run_id":"logged-in","sequence":2,"time":"2026-01-05T09:00:00Z"}','8d766c16cb399191a3a62a7aedb3152023d7a951b05a40a3486efe34f1c1569f');
INSERT INTO "entries" VALUES(3,'{"actor":{"kind":"person","name":"os_user"},"details":{"accepted":"sha256/3f2a6c8a3e1b7f9d5c4e2a1b0c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c1d","host":"intranet.example.com","previous":"sha256/ad0c5cd9a82e8a1ecbd6e9a7a8f7a3b4bd8f4a3c1c8c20c3b0d0bbc6bc6a4f17"},"kind":"certificate_change_accepted","sequence":3,"surface":"cli","time":"2026-01-05T09:00:00Z"}','3779d15d2fd174c39fbccd5539315086efd889da7a41c3d3705faab1f1b5f2fe');
INSERT INTO "entries" VALUES(4,'{"actor":{"kind":"engine"},"details":{"setting":"security.audit_chain","value":false},"kind":"security_switch_turned_off","layer":"run","run_id":"unchained","sequence":4,"time":"2026-01-05T09:00:00Z"}','100f9fa92de5db57d12f5f88903f21a5763f8049ce59413b2e95623fe0fc65d3');
INSERT INTO "entries" VALUES(5,'{"actor":{"kind":"engine"},"chained":false,"details":{"domain":null,"placeholder":"run_state.key_reference","reference":"env:RUN_KEY"},"kind":"secret_reference_used","layer":"run","run_id":"unchained","sequence":5,"time":"2026-01-05T09:00:00Z"}','b1eb609d87be6ae506629eeb98b5cc9d6ba1d40c4c1b191792f6148b0b48cd22');
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('audit-log','0.1');
COMMIT;
