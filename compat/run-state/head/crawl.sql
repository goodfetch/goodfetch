PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE attempt_errors (
            work_item TEXT NOT NULL,
            attempt INTEGER NOT NULL,
            code TEXT NOT NULL,
            message TEXT NOT NULL,
            details TEXT,
            PRIMARY KEY (work_item, attempt)
        ) STRICT;
INSERT INTO "attempt_errors" VALUES('2',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
CREATE TABLE bounds (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_depth INTEGER,
            max_requests INTEGER,
            max_url_length INTEGER,
            max_path_segments INTEGER,
            max_segment_repeats INTEGER
        ) STRICT;
INSERT INTO "bounds" VALUES(1,3,100,2083,20,2);
CREATE TABLE bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT;
INSERT INTO "bounds_admitted" VALUES(1,5);
CREATE TABLE bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "bounds_refusals" VALUES('crawl.max_segment_repeats',1);
CREATE TABLE domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT;
INSERT INTO "domains" VALUES('shop.test',1767603605299,0);
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT NOT NULL) STRICT;
CREATE TABLE outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT;
INSERT INTO "outcomes" VALUES(1,'3','succeeded','home',1,'{"position":1,"request":{"id":"3","url":"https://api.test/search","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:00Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(2,'4','blocked','guarded',1,'{"position":2,"request":{"id":"4","url":"https://guarded.test/","label":"guarded","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:00Z","kind":"blocked","cause":"ip_block"}');
INSERT INTO "outcomes" VALUES(3,'2','failed','product',1,'{"position":3,"request":{"id":"2","url":"https://shop.test/broken","label":"product","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:00Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(4,'1','succeeded','home',1,'{"position":4,"request":{"id":"1","url":"https://shop.test/","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:00Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(5,'5','succeeded','home',1,'{"position":5,"request":{"id":"5","url":"https://outlet.test/sale","label":"home","depth":1},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:00Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
CREATE TABLE page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL) STRICT;
CREATE TABLE pool_sessions (
            position INTEGER NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT
        ) STRICT;
INSERT INTO "pool_sessions" VALUES(1,'none',1767603600000,1,0.0,NULL);
INSERT INTO "pool_sessions" VALUES(2,'none',1767603600000,1,0.0,NULL);
INSERT INTO "pool_sessions" VALUES(3,'none',1767603600000,1,1.0,'escalated');
INSERT INTO "pool_sessions" VALUES(4,'none',1767603600000,1,0.0,NULL);
INSERT INTO "pool_sessions" VALUES(5,'none',1767603600000,1,0.0,NULL);
CREATE TABLE pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL
        ) STRICT;
INSERT INTO "pool_tiers" VALUES('guarded.test','datacenter');
CREATE TABLE records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
INSERT INTO "records" VALUES('pages',1,'3','{"title":"Shop","url":"https://shop.test/"}');
INSERT INTO "records" VALUES('links',1,'3','{"href":"https://shop.test/broken"}');
INSERT INTO "records" VALUES('pages',2,'1','{"title":"Shop","url":"https://shop.test/"}');
INSERT INTO "records" VALUES('links',2,'1','{"href":"https://shop.test/broken"}');
INSERT INTO "records" VALUES('pages',3,'5','{"title":"Shop","url":"https://shop.test/"}');
INSERT INTO "records" VALUES('links',3,'5','{"href":"https://shop.test/broken"}');
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
        weakened TEXT NOT NULL DEFAULT '[]',
        journal_position INTEGER NOT NULL DEFAULT 0,
        binding TEXT,
        clean_up_digest TEXT,
        uncapped INTEGER NOT NULL DEFAULT 0,
        progress_message TEXT,
        nothing_done TEXT,
        report TEXT
    ) STRICT;
INSERT INTO "run" VALUES('crawl','crawl','foxhound','os_user',1,'{"seeds":[{"label":"home","payload":{"depth":0},"url":"https://shop.test/"},{"label":"product","url":"https://shop.test/broken"},{"label":"home","url":"https://SHOP.test:443/"},{"body":"eyJwYWdlIjoxfQ==","key":"search-1","label":"home","method":"POST","url":"https://api.test/search"},{"label":"guarded","url":"https://guarded.test/"},{"label":"home","url":"https://shop.test/more/more/more/"}]}','finished',NULL,'verified','unverified','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z','sha256:0227dc879fda140d','[]',42,NULL,NULL,0,NULL,NULL,'{"run_id":"crawl","lifecycle":"finished","verdict":"verified","completeness":"unverified","completeness_reasons":[{"code":"crawl.max_segment_repeats","message":"`crawl.max_segment_repeats` refused 1 URL the crawl would otherwise have admitted."}],"started_at":"2026-01-05T09:00:00Z","ended_at":"2026-01-05T09:00:00Z","usage":{"model":{"calls":0,"tokens":0},"browser_time":0,"proxy_bytes":0,"requests":5,"limits":[{"scope":{"kind":"install","id":""},"usage_kind":"requests","amount":1000,"left":995,"set_by":"client"}]},"policy_version":"sha256:0227dc879fda140d","records":[{"committed":3,"dataset":"links","dropped":0,"rejected":0},{"committed":3,"dataset":"pages","dropped":0,"rejected":0}],"requests":{"blocked":1,"by_block_cause":[{"count":1,"name":"ip_block"}],"by_error_code":[{"count":1,"name":"deadline_exceeded"}],"failed":1,"pending":0,"skipped":0,"succeeded":3}}');
CREATE TABLE scope (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            strategy TEXT NOT NULL,
            include TEXT NOT NULL,
            exclude TEXT NOT NULL
        ) STRICT;
INSERT INTO "scope" VALUES(1,'all','["**/sale"]','["**/admin/**"]');
CREATE TABLE scope_seeds (
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            scheme TEXT NOT NULL,
            domain TEXT NOT NULL,
            PRIMARY KEY (host, port, scheme)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "scope_seeds" VALUES('api.test',0,'https','api.test');
INSERT INTO "scope_seeds" VALUES('guarded.test',0,'https','guarded.test');
INSERT INTO "scope_seeds" VALUES('shop.test',0,'https','shop.test');
CREATE TABLE seeds (
            position INTEGER NOT NULL PRIMARY KEY,
            answer TEXT NOT NULL
        ) STRICT;
INSERT INTO "seeds" VALUES(1,'{"kind":"added","id":"1","url":"https://shop.test/"}');
INSERT INTO "seeds" VALUES(2,'{"kind":"added","id":"2","url":"https://shop.test/broken"}');
INSERT INTO "seeds" VALUES(3,'{"kind":"not_added","reason":"duplicate","url":"https://SHOP.test:443/"}');
INSERT INTO "seeds" VALUES(4,'{"kind":"added","id":"3","url":"https://api.test/search"}');
INSERT INTO "seeds" VALUES(5,'{"kind":"added","id":"4","url":"https://guarded.test/"}');
INSERT INTO "seeds" VALUES(6,'{"kind":"not_added","reason":"max_segment_repeats","url":"https://shop.test/more/more/more/"}');
CREATE TABLE tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT;
INSERT INTO "tags" VALUES(0,'schedule:nightly');
CREATE TABLE usage (
        slice_id INTEGER NOT NULL,
        epoch INTEGER NOT NULL,
        used INTEGER NOT NULL,
        PRIMARY KEY (slice_id, epoch)
    ) STRICT;
INSERT INTO "usage" VALUES(1,1,5);
CREATE TABLE usage_totals (
        usage_kind TEXT NOT NULL,
        currency TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, currency)
    ) STRICT;
INSERT INTO "usage_totals" VALUES('requests','',5);
CREATE TABLE visited (
            identity TEXT NOT NULL PRIMARY KEY,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "visited" VALUES('GET https://guarded.test/','4');
INSERT INTO "visited" VALUES('GET https://outlet.test/sale','5');
INSERT INTO "visited" VALUES('GET https://shop.test/','1');
INSERT INTO "visited" VALUES('GET https://shop.test/broken','2');
INSERT INTO "visited" VALUES('key:search-1','3');
CREATE TABLE warnings (
        position INTEGER NOT NULL PRIMARY KEY,
        code TEXT NOT NULL,
        message TEXT NOT NULL,
        sessions TEXT NOT NULL,
        folder TEXT
    ) STRICT;
CREATE TABLE work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            method TEXT NOT NULL,
            body BLOB,
            label TEXT NOT NULL,
            depth INTEGER NOT NULL,
            payload TEXT,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            lease_epoch INTEGER NOT NULL,
            error_code TEXT,
            domain_key TEXT,
            priority INTEGER NOT NULL,
            forefront INTEGER
        ) STRICT;
INSERT INTO "work_items" VALUES('1',1,'https://shop.test/','GET',NULL,'home',0,'{"depth":0}','succeeded',1,1,NULL,'shop.test',0,NULL);
INSERT INTO "work_items" VALUES('2',2,'https://shop.test/broken','GET',NULL,'product',0,NULL,'failed',1,1,'deadline_exceeded','shop.test',0,NULL);
INSERT INTO "work_items" VALUES('3',3,'https://api.test/search','POST',X'7B2270616765223A317D','home',0,NULL,'succeeded',1,1,NULL,'api.test',0,NULL);
INSERT INTO "work_items" VALUES('4',4,'https://guarded.test/','GET',NULL,'guarded',0,NULL,'blocked',1,1,NULL,'guarded.test',0,NULL);
INSERT INTO "work_items" VALUES('5',5,'https://outlet.test/sale','GET',NULL,'home',1,NULL,'succeeded',1,1,NULL,'outlet.test',0,5);
CREATE INDEX outcomes_by_work_item ON outcomes (work_item, position);
CREATE INDEX pool_sessions_open ON pool_sessions (position)
            WHERE retired_because IS NULL;
CREATE INDEX scope_seeds_by_domain ON scope_seeds (domain);
CREATE INDEX work_items_by_domain
            ON work_items (status, domain_key, forefront DESC, priority DESC, position);
CREATE INDEX work_items_by_status ON work_items (status, position);
CREATE INDEX work_items_unkeyed ON work_items (position)
            WHERE domain_key IS NULL;
COMMIT;
