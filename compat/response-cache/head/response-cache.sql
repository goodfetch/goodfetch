PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE bodies (
        sha256 TEXT PRIMARY KEY,
        size INTEGER NOT NULL,
        holders INTEGER NOT NULL
    ) STRICT;
INSERT INTO "bodies" VALUES('04d8aa218b6b2af426e127aafe751f3451e67fdd7607b3afb61a60f653f1ebc5',20,1);
INSERT INTO "bodies" VALUES('1c42f1572658c2ae95e3090425e1c96f49daa975cc503fc6e513f224d05b0faa',17,1);
INSERT INTO "bodies" VALUES('5021e624e752b001ce3e3846e8f158ed4aeb93a4c9a72fdb35a0c5b14a0eea84',14,1);
CREATE TABLE entries (
        partition TEXT NOT NULL,
        person TEXT NOT NULL,
        country TEXT NOT NULL,
        language TEXT NOT NULL,
        device TEXT NOT NULL,
        method TEXT NOT NULL,
        url TEXT NOT NULL,
        vary_key TEXT NOT NULL,
        replay_key TEXT NOT NULL,
        request_headers TEXT NOT NULL,
        status INTEGER NOT NULL,
        response_headers TEXT NOT NULL,
        fetched_at TEXT NOT NULL,
        body_sha256 TEXT NOT NULL,
        body_size INTEGER NOT NULL,
        run_id TEXT,
        used_at TEXT NOT NULL,
        PRIMARY KEY (partition, person, country, language, device, method, url, vary_key, replay_key)
    ) STRICT;
INSERT INTO "entries" VALUES('public','os_user','','','','GET','https://cache.test/revalidate','37cd5d7393ea04b36e98c73b66961dcbf19cc89cb152e66ed098f7be6ca5e33e','','[["accept-language","en"]]',200,'[["content-type","text/html"],["cache-control","max-age=60"],["vary","Accept-Language"],["etag","\"v1\""],["last-modified","Sun, 04 Jan 2026 09:00:00 GMT"]]','2026-01-05T09:00:00.000Z','04d8aa218b6b2af426e127aafe751f3451e67fdd7607b3afb61a60f653f1ebc5',20,NULL,'2026-01-05T09:00:00.000Z');
INSERT INTO "entries" VALUES('public','grace','FR','fr-FR','mobile','GET','https://cache.test/fresh','e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855','','[]',200,'[["cache-control","max-age=3600"]]','2026-01-05T09:00:00.000Z','1c42f1572658c2ae95e3090425e1c96f49daa975cc503fc6e513f224d05b0faa',17,'filled-the-cache','2026-01-05T09:00:00.000Z');
INSERT INTO "entries" VALUES('public','os_user','','','','POST','https://cache.test/api/search','e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855','6ada69eaa05d60fd8c41f6fda66091310358c2c92aaeb745425a9f694574aa38','[]',200,'[["content-type","application/json"],["cache-control","no-store"]]','2026-01-05T09:00:00.000Z','5021e624e752b001ce3e3846e8f158ed4aeb93a4c9a72fdb35a0c5b14a0eea84',14,'filled-the-cache','2026-01-05T09:00:00.000Z');
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('response-cache','0.1');
CREATE TABLE held (
        one INTEGER PRIMARY KEY CHECK (one = 1),
        bytes INTEGER NOT NULL
    ) STRICT;
INSERT INTO "held" VALUES(1,51);
CREATE INDEX bodies_unheld ON bodies (sha256) WHERE holders = 0;
CREATE INDEX entries_by_fetch ON entries (fetched_at);
CREATE INDEX entries_by_use ON entries (used_at);
COMMIT;
