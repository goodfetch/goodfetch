PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('accounts-store','0.1');
CREATE TABLE sign_ins (
        id TEXT NOT NULL PRIMARY KEY,
        verifier TEXT NOT NULL,
        person TEXT NOT NULL,
        created TEXT NOT NULL,
        last_used TEXT NOT NULL,
        wrapping_key TEXT NOT NULL
    ) STRICT;
INSERT INTO "sign_ins" VALUES('8a6f0c1e5d2b4a7f9e3c1b0d2a4f6e8c','3c1f6b2e8d0a4c7e9f1b3d5a7c9e1f2b4d6a8c0e2f4b6d8a0c2e4f6b8d0a2c4e','os_user','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z','5e7a9c1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5c7e9b1d3f5a7c');
INSERT INTO "sign_ins" VALUES('2d4f6a8c0e1b3d5f7a9c1e3b5d7f9a1c','9e7c5a3f1d9b7e5c3a1f9d7b5e3c1a9f7d5b3e1c9a7f5d3b1e9c7a5f3d1b9e7c','os_user','2026-01-05T09:00:00Z','2026-01-05T10:00:00Z','1b3d5f7a9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d');
COMMIT;
