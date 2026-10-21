PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE attempt_errors (
            work_item TEXT NOT NULL,
            attempt INTEGER NOT NULL,
            code TEXT NOT NULL,
            message TEXT NOT NULL,
            PRIMARY KEY (work_item, attempt)
        ) STRICT;
INSERT INTO "attempt_errors" VALUES('2',1,'handler_failed','a bug');
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT;
INSERT INTO "outcomes" VALUES(1,'1','succeeded','home',1,'{"position":1,"request":{"id":"1","url":"https://shop.test/","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:00Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(2,'2','failed','product',1,'{"position":2,"request":{"id":"2","url":"https://shop.test/broken","label":"product","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"handler_failed","message":"a bug"}],"ended_at":"2026-01-05T09:00:00Z","kind":"failed","error":{"attempt":1,"code":"handler_failed","message":"a bug"}}');
CREATE TABLE records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
INSERT INTO "records" VALUES('pages',1,'1','{"title":"Shop","url":"https://shop.test/"}');
INSERT INTO "records" VALUES('links',1,'1','{"href":"https://shop.test/broken"}');
CREATE TABLE run (
        run_id TEXT NOT NULL PRIMARY KEY,
        kind TEXT NOT NULL,
        needs TEXT NOT NULL,
        caller TEXT NOT NULL,
        epoch INTEGER NOT NULL,
        inputs TEXT NOT NULL,
        lifecycle TEXT NOT NULL,
        stop_reason TEXT,
        verdict TEXT,
        completeness TEXT,
        started_at TEXT NOT NULL,
        ended_at TEXT,
        policy_version TEXT,
        weakened_by TEXT NOT NULL DEFAULT '[]'
    ) STRICT;
INSERT INTO "run" VALUES('crawl','crawl','foxhound','os_user',1,'{"seeds":[{"label":"home","payload":{"depth":0},"url":"https://shop.test/"},{"label":"product","url":"https://shop.test/broken"}]}','finished',NULL,'verified','complete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z','sha256:0227dc879fda140d','[]');
CREATE TABLE tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT;
INSERT INTO "tags" VALUES(0,'schedule:nightly');
CREATE TABLE work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            label TEXT NOT NULL,
            depth INTEGER NOT NULL,
            payload TEXT,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            lease_epoch INTEGER NOT NULL,
            error_code TEXT
        ) STRICT;
INSERT INTO "work_items" VALUES('1',1,'https://shop.test/','home',0,'{"depth":0}','succeeded',1,1,NULL);
INSERT INTO "work_items" VALUES('2',2,'https://shop.test/broken','product',0,NULL,'failed',1,1,'handler_failed');
CREATE INDEX outcomes_by_work_item ON outcomes (work_item, position);
CREATE INDEX work_items_by_status ON work_items (status, position);
COMMIT;
