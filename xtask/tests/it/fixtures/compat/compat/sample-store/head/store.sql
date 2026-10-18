PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE runs (
  id INTEGER PRIMARY KEY,
  url TEXT NOT NULL,
  finished_at TEXT,
  CONSTRAINT url_unique UNIQUE (url)
);
INSERT INTO runs VALUES(1,'https://example.invalid/a;b','2026-01-01T00:00:00Z');
INSERT INTO runs VALUES(2,'https://example.invalid/it''s',NULL);
CREATE TABLE IF NOT EXISTS "pages" ("run" INTEGER NOT NULL, "body" BLOB, PRIMARY KEY ("run"));
INSERT INTO pages VALUES(1,X'00ff');
COMMIT;
