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
INSERT INTO "attempt_errors" VALUES('5',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('2',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('6',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('7',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('8',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('9',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('10',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('11',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('12',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('13',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('15',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
INSERT INTO "attempt_errors" VALUES('16',1,'deadline_exceeded','no first byte','{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}');
CREATE TABLE bounds (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_depth INTEGER,
            max_requests INTEGER,
            max_url_length INTEGER,
            max_path_segments INTEGER,
            max_segment_repeats INTEGER,
            max_sitemap_nesting INTEGER,
            learning INTEGER NOT NULL
        ) STRICT;
INSERT INTO "bounds" VALUES(1,3,100,2083,20,2,5,1);
CREATE TABLE bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT;
INSERT INTO "bounds_admitted" VALUES(1,21);
CREATE TABLE bounds_path_budgets (
            position INTEGER NOT NULL PRIMARY KEY,
            pattern TEXT NOT NULL,
            max_requests INTEGER NOT NULL
        ) STRICT;
INSERT INTO "bounds_path_budgets" VALUES(0,'/pages/*',1);
CREATE TABLE bounds_path_counts (
            pattern TEXT NOT NULL PRIMARY KEY,
            admitted INTEGER NOT NULL,
            refused INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "bounds_path_counts" VALUES('/pages/*',1,1);
CREATE TABLE bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "bounds_refusals" VALUES('crawl.max_segment_repeats',1);
CREATE TABLE code_free_extraction (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            extraction TEXT NOT NULL
        ) STRICT;
CREATE TABLE datasets (
            name TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL,
            schema TEXT,
            schema_hash TEXT,
            key TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "datasets" VALUES('links',1,NULL,NULL,'[]');
INSERT INTO "datasets" VALUES('pages',0,'{"properties":{"title":{"type":"string"},"url":{"type":"string"}},"required":["url"],"type":"object"}','0cc5066123c66c6b05b623ef251d6f0ecc72c6335a787255f2d42c7dfb19f1a4','["url"]');
CREATE TABLE domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT;
INSERT INTO "domains" VALUES('list.test',1767603656489,0);
INSERT INTO "domains" VALUES('shop.test',1767603606732,0);
CREATE TABLE effect_journal (entry INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, key TEXT NOT NULL, repeatable INTEGER NOT NULL, state TEXT NOT NULL CHECK (state IN ('open', 'sent', 'done', 'uncertain', 'reconciled')), output TEXT, reason TEXT, first_command TEXT, since TEXT, performed_at TEXT, reconciled_by TEXT, reconciled_at TEXT, UNIQUE (namespace, key), CHECK (state NOT IN ('sent', 'uncertain') OR since IS NOT NULL), CHECK (state <> 'done' OR performed_at IS NOT NULL), CHECK (state <> 'uncertain' OR reason IS NOT NULL), CHECK (state <> 'reconciled' OR (reconciled_by IS NOT NULL AND reconciled_at IS NOT NULL))) STRICT;
CREATE TABLE effect_secret (id INTEGER PRIMARY KEY CHECK (id = 1), secret BLOB NOT NULL) STRICT;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT, content BLOB, content_type TEXT, file INTEGER, CHECK ((value IS NULL) <> (content_type IS NULL)), CHECK (content_type IS NULL OR ((content IS NULL) <> (file IS NULL)))) STRICT;
CREATE TABLE labels (
            name TEXT NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT;
INSERT INTO "labels" VALUES('home','{"payload":{"type":["object","null"]}}');
INSERT INTO "labels" VALUES('product','{}');
CREATE TABLE learned_identities (
            identity TEXT NOT NULL PRIMARY KEY,
            rule INTEGER NOT NULL,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "learned_identities" VALUES('GET https://list.test/items?n=1',1,'6');
INSERT INTO "learned_identities" VALUES('GET https://list.test/items?n=2',1,'8');
INSERT INTO "learned_identities" VALUES('GET https://list.test/items?n=3',1,'10');
CREATE TABLE learned_merges (
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            url TEXT NOT NULL,
            identity TEXT,
            UNIQUE (rule, url)
        ) STRICT;
INSERT INTO "learned_merges" VALUES(1,'https://list.test/items?n=3&ref=d',NULL);
INSERT INTO "learned_merges" VALUES(1,'https://list.test/items?n=1&ref=z','GET https://list.test/items?n=1&ref=z');
CREATE TABLE learned_rules (
            id INTEGER NOT NULL PRIMARY KEY,
            kind TEXT NOT NULL,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            form TEXT NOT NULL,
            evidence TEXT NOT NULL,
            contradiction TEXT,
            UNIQUE (kind, host, path, parameter, form)
        ) STRICT;
INSERT INTO "learned_rules" VALUES(1,'no_effect_parameter','list.test','','ref','','3 pairs of URLs differing only by `ref` served pages of matching signatures, and no pair pages of differing ones: https://list.test/items?n=1 and https://list.test/items?n=1&ref=a; https://list.test/items?n=2 and https://list.test/items?n=2&ref=b; https://list.test/items?n=3 and https://list.test/items?n=3&ref=c.',NULL);
INSERT INTO "learned_rules" VALUES(2,'equivalent_form','list.test','','','trailing_slash','https://list.test/guide redirected to https://list.test/guide/, which differs from it only by a trailing slash.','https://list.test/guide and https://list.test/guide/, both fetched and neither redirected, served pages of differing signatures.');
INSERT INTO "learned_rules" VALUES(3,'cursor_pagination','list.test','/rows','offset','','`offset` steps by 20 on https://list.test/rows (https://list.test/rows?offset=0, https://list.test/rows?offset=20, https://list.test/rows?offset=40), and three of its pages each link one value of `after` of their own: https://list.test/rows?offset=0 links https://list.test/rows?after=r20/sale, https://list.test/rows?offset=20 links https://list.test/rows?after=r40/sale, and https://list.test/rows?offset=40 links https://list.test/rows?after=r60/sale.',NULL);
CREATE TABLE outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT;
INSERT INTO "outcomes" VALUES(1,'3','succeeded','home',1,'{"position":1,"request":{"id":"3","url":"https://api.test/search","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:02Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(2,'4','blocked','guarded',1,'{"position":2,"request":{"id":"4","url":"https://guarded.test/","label":"guarded","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:02Z","kind":"blocked","cause":"ip_block"}');
INSERT INTO "outcomes" VALUES(3,'5','failed','listed',1,'{"position":3,"request":{"id":"5","url":"https://list.test/items?n=1","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(4,'2','failed','product',1,'{"position":4,"request":{"id":"2","url":"https://shop.test/broken","label":"product","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(5,'14','succeeded','home',1,'{"position":5,"request":{"id":"14","url":"https://shop.test/pages/1","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:02Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(6,'6','failed','listed',1,'{"position":6,"request":{"id":"6","url":"https://list.test/items?n=1&ref=a","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(7,'7','failed','listed',1,'{"position":7,"request":{"id":"7","url":"https://list.test/items?n=2","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(8,'8','failed','listed',1,'{"position":8,"request":{"id":"8","url":"https://list.test/items?n=2&ref=b","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(9,'9','failed','listed',1,'{"position":9,"request":{"id":"9","url":"https://list.test/items?n=3","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(10,'10','failed','listed',1,'{"position":10,"request":{"id":"10","url":"https://list.test/items?n=3&ref=c","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(11,'11','failed','listed',1,'{"position":11,"request":{"id":"11","url":"https://list.test/rows?offset=0","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(12,'12','failed','listed',1,'{"position":12,"request":{"id":"12","url":"https://list.test/rows?offset=20","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(13,'13','failed','listed',1,'{"position":13,"request":{"id":"13","url":"https://list.test/rows?offset=40","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(14,'15','failed','listed',1,'{"position":14,"request":{"id":"15","url":"https://list.test/guide","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(15,'16','failed','listed',1,'{"position":15,"request":{"id":"16","url":"https://list.test/guide/","label":"listed","depth":0},"attempts":1,"errors":[{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}],"ended_at":"2026-01-05T09:00:02Z","kind":"failed","error":{"attempt":1,"code":"deadline_exceeded","message":"no first byte","details":{"elapsed":30000,"limit":{"key":"timeout.first_byte","layer":"default","value":30000},"waited_for":"first_byte"}}}');
INSERT INTO "outcomes" VALUES(16,'1','succeeded','home',1,'{"position":16,"request":{"id":"1","url":"https://shop.test/","label":"home","depth":0},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:02Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":1},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
INSERT INTO "outcomes" VALUES(17,'23','succeeded','home',1,'{"position":17,"request":{"id":"23","url":"https://outlet.test/sale","label":"home","depth":1},"attempts":1,"errors":[],"ended_at":"2026-01-05T09:00:04Z","kind":"succeeded","records":[{"dataset":"pages","committed":1,"dropped":0,"rejected":0},{"dataset":"links","committed":1,"dropped":0,"rejected":0}]}');
CREATE TABLE page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL, source BLOB) STRICT;
CREATE TABLE payload_cap (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_size INTEGER NOT NULL
        ) STRICT;
INSERT INTO "payload_cap" VALUES(1,65536);
CREATE TABLE payload_labels (
            label TEXT NOT NULL PRIMARY KEY,
            schema TEXT,
            hash TEXT
        ) STRICT, WITHOUT ROWID;
INSERT INTO "payload_labels" VALUES('home','{"type":["object","null"]}','5d4827562aefcfb1c3413607ff8ccbcce66217250c6ee5c6565bd52ef9eebc64');
INSERT INTO "payload_labels" VALUES('product',NULL,NULL);
CREATE TABLE payloads (
            request TEXT NOT NULL PRIMARY KEY,
            payload TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "payloads" VALUES('1','{"depth":0}');
CREATE TABLE pool_blocked_address_blocks (
            domain_key TEXT NOT NULL,
            block TEXT NOT NULL,
            PRIMARY KEY (domain_key, block)
        ) STRICT;
INSERT INTO "pool_blocked_address_blocks" VALUES('guarded.test','ipv4/24:8323072');
CREATE TABLE pool_sessions (
            pool TEXT NOT NULL,
            position INTEGER NOT NULL,
            proxy_tier TEXT NOT NULL,
            sends_from TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT,
            address_ordinal INTEGER,
            PRIMARY KEY (pool, position)
        ) STRICT;
INSERT INTO "pool_sessions" VALUES('',1,'none','transport',1767603600000,1,0.0,NULL,0);
INSERT INTO "pool_sessions" VALUES('',2,'none','transport',1767603600000,1,0.0,NULL,1);
INSERT INTO "pool_sessions" VALUES('',3,'none','transport',1767603600000,1,1.0,'escalated',2);
INSERT INTO "pool_sessions" VALUES('',4,'none','transport',1767603600000,1,0.0,NULL,3);
INSERT INTO "pool_sessions" VALUES('',5,'none','transport',1767603600000,1,0.0,NULL,4);
INSERT INTO "pool_sessions" VALUES('',6,'none','transport',1767603600000,1,0.0,NULL,5);
INSERT INTO "pool_sessions" VALUES('',7,'none','transport',1767603600000,1,0.0,NULL,6);
INSERT INTO "pool_sessions" VALUES('',8,'none','transport',1767603600000,1,0.0,NULL,7);
INSERT INTO "pool_sessions" VALUES('',9,'none','transport',1767603600000,1,0.0,NULL,8);
INSERT INTO "pool_sessions" VALUES('',10,'none','transport',1767603600000,1,0.0,NULL,9);
INSERT INTO "pool_sessions" VALUES('',11,'none','transport',1767603600000,1,0.0,NULL,10);
INSERT INTO "pool_sessions" VALUES('',12,'none','transport',1767603600000,1,0.0,NULL,11);
INSERT INTO "pool_sessions" VALUES('',13,'none','transport',1767603600000,1,0.0,NULL,12);
INSERT INTO "pool_sessions" VALUES('',14,'none','transport',1767603600000,1,0.0,NULL,13);
INSERT INTO "pool_sessions" VALUES('',15,'none','transport',1767603600000,1,0.0,NULL,14);
INSERT INTO "pool_sessions" VALUES('',16,'none','transport',1767603600000,1,0.0,NULL,15);
INSERT INTO "pool_sessions" VALUES('',17,'none','transport',1767603602000,1,0.0,NULL,16);
CREATE TABLE pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            browser INTEGER NOT NULL
        ) STRICT;
INSERT INTO "pool_tiers" VALUES('guarded.test','datacenter',0);
CREATE TABLE record_counts (
            dataset TEXT NOT NULL PRIMARY KEY,
            committed INTEGER NOT NULL,
            dropped INTEGER NOT NULL,
            rejected INTEGER NOT NULL
        ) STRICT;
INSERT INTO "record_counts" VALUES('pages',4,0,1);
INSERT INTO "record_counts" VALUES('links',4,0,0);
CREATE TABLE records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            url TEXT NOT NULL,
            committed_at TEXT NOT NULL,
            data TEXT NOT NULL,
            schema_hash TEXT,
            provenance TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
INSERT INTO "records" VALUES('pages',1,'3','https://api.test/search','2026-01-05T09:00:02Z','{"title":"Shop","url":"https://shop.test/"}','0cc5066123c66c6b05b623ef251d6f0ecc72c6335a787255f2d42c7dfb19f1a4','{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"3","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('links',1,'3','https://api.test/search','2026-01-05T09:00:02Z','{"href":"https://shop.test/broken"}',NULL,'{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"3","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('pages',2,'14','https://shop.test/pages/1','2026-01-05T09:00:02Z','{"title":"Shop","url":"https://shop.test/"}','0cc5066123c66c6b05b623ef251d6f0ecc72c6335a787255f2d42c7dfb19f1a4','{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"14","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('links',2,'14','https://shop.test/pages/1','2026-01-05T09:00:02Z','{"href":"https://shop.test/broken"}',NULL,'{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"14","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('pages',3,'1','https://shop.test/','2026-01-05T09:00:02Z','{"title":"Shop","url":"https://shop.test/"}','0cc5066123c66c6b05b623ef251d6f0ecc72c6335a787255f2d42c7dfb19f1a4','{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"1","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('links',3,'1','https://shop.test/','2026-01-05T09:00:02Z','{"href":"https://shop.test/broken"}',NULL,'{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"1","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('pages',4,'23','https://outlet.test/sale','2026-01-05T09:00:04Z','{"title":"Shop","url":"https://shop.test/"}','0cc5066123c66c6b05b623ef251d6f0ecc72c6335a787255f2d42c7dfb19f1a4','{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"23","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
INSERT INTO "records" VALUES('links',4,'23','https://outlet.test/sale','2026-01-05T09:00:04Z','{"href":"https://shop.test/broken"}',NULL,'{"clean_up_rules":"sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9","encoding":{"encoding":"UTF-8","source":"header"},"request_id":"23","tier":{"chosen":"forced","forced_by":"run","tier":"http"},"value_transforms":[]}');
CREATE TABLE rejected_records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            rejection TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
INSERT INTO "rejected_records" VALUES('pages',1,'1','{"title":"No address"}','{"issues":[{"message":"\"url\" is a required property","path":""}],"reason":"A Record of the Dataset `pages` does not hold, so it was not staged: \"url\" is a required property"}');
CREATE TABLE request_counts (
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (kind, name)
        ) STRICT;
INSERT INTO "request_counts" VALUES('succeeded','',4);
INSERT INTO "request_counts" VALUES('blocked','ip_block',1);
INSERT INTO "request_counts" VALUES('failed','deadline_exceeded',12);
CREATE TABLE robots_txt_copies (location TEXT NOT NULL PRIMARY KEY, status INTEGER, body BLOB, unreachable TEXT, CHECK ((status IS NULL) = (body IS NULL)), CHECK ((status IS NULL) <> (unreachable IS NULL))) STRICT;
CREATE TABLE rules_canonicals (
            host TEXT NOT NULL,
            form TEXT NOT NULL,
            url TEXT NOT NULL,
            canonical TEXT NOT NULL,
            PRIMARY KEY (host, form, url)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_canonicals" VALUES('list.test','scheme','https://list.test/items?n=1','http://list.test/items?n=1');
CREATE TABLE rules_cursors (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value TEXT NOT NULL,
            page TEXT NOT NULL,
            url TEXT NOT NULL,
            second_value TEXT,
            second_page TEXT,
            second_url TEXT,
            refuted INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_cursors" VALUES('list.test','/rows','after','r20/sale','https://list.test/rows?offset=0','https://list.test/rows?after=r20/sale','r40/sale','https://list.test/rows?offset=20','https://list.test/rows?after=r40/sale',0);
CREATE TABLE rules_ended (
            crawl_request TEXT NOT NULL,
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            PRIMARY KEY (crawl_request, rule)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_ended" VALUES('17',1);
CREATE TABLE rules_offset_values (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value INTEGER NOT NULL,
            url TEXT NOT NULL,
            PRIMARY KEY (host, path, parameter, value)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_offset_values" VALUES('list.test','/items','n',1,'https://list.test/items?n=1');
INSERT INTO "rules_offset_values" VALUES('list.test','/items','n',2,'https://list.test/items?n=2');
INSERT INTO "rules_offset_values" VALUES('list.test','/rows','offset',0,'https://list.test/rows?offset=0');
INSERT INTO "rules_offset_values" VALUES('list.test','/rows','offset',20,'https://list.test/rows?offset=20');
INSERT INTO "rules_offset_values" VALUES('list.test','/rows','offset',40,'https://list.test/rows?offset=40');
CREATE TABLE rules_offsets (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            anchor INTEGER NOT NULL,
            step INTEGER NOT NULL,
            low INTEGER NOT NULL,
            high INTEGER NOT NULL,
            count INTEGER NOT NULL,
            paged INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_offsets" VALUES('list.test','/items','n',1,1,1,2,2,0);
INSERT INTO "rules_offsets" VALUES('list.test','/rows','offset',0,20,0,40,3,1);
CREATE TABLE rules_pages (
            crawl_request TEXT NOT NULL PRIMARY KEY,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            query TEXT NOT NULL,
            url TEXT NOT NULL,
            signature BLOB NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_pages" VALUES('1','shop.test','/','','https://shop.test/',X'E5ADAA0D5A50ACDD9DB6B0DFE44539278670AAF516E549CB75C7D7205EF87EC8');
INSERT INTO "rules_pages" VALUES('10','list.test','/items','n=3&ref=c','https://list.test/items?n=3&ref=c',X'3333333333333333333333333333333333333333333333333333333333333333');
INSERT INTO "rules_pages" VALUES('11','list.test','/rows','offset=0','https://list.test/rows?offset=0',X'BD5CC37D2B90B36FDBB5AC71E8B9B54089950B1F205EC4B7E39C7FC2124ABA39');
INSERT INTO "rules_pages" VALUES('12','list.test','/rows','offset=20','https://list.test/rows?offset=20',X'DA9F555019388929252426958FB762AD8F362704D5B726EAA8A8C71633CBD287');
INSERT INTO "rules_pages" VALUES('13','list.test','/rows','offset=40','https://list.test/rows?offset=40',X'C0824ED446F6456FF101EF50F00BE6A7D66B65D00E521AD697138B090D0FBB6F');
INSERT INTO "rules_pages" VALUES('14','shop.test','/pages/1','','https://shop.test/pages/1',X'02EC18F1FB312DAD2BFDECB2306A105221BCD8686CD656D6F4D18BD81C07D991');
INSERT INTO "rules_pages" VALUES('15','list.test','/guide','','https://list.test/guide',X'6767676767676767676767676767676767676767676767676767676767676767');
INSERT INTO "rules_pages" VALUES('16','list.test','/guide/','','https://list.test/guide/',X'6868686868686868686868686868686868686868686868686868686868686868');
INSERT INTO "rules_pages" VALUES('2','shop.test','/broken','','https://shop.test/broken',X'4BFF38ABCDC6277E34168C3B3C0C5DD0B2B4E82EB9449D0DB143004BFCBAAAFD');
INSERT INTO "rules_pages" VALUES('23','outlet.test','/sale','','https://outlet.test/sale',X'12AFC63172B9490AD94BAD409108B6621E3F366FAC138B4617C6A18139BD5D0F');
INSERT INTO "rules_pages" VALUES('4','guarded.test','/','','https://guarded.test/',X'0DB7F1157A3804BAAABB5F555AD30FF56B33C2863FE35359A8A869DA600F938F');
INSERT INTO "rules_pages" VALUES('5','list.test','/items','n=1','https://list.test/items?n=1',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_pages" VALUES('6','list.test','/items','n=1&ref=a','https://list.test/items?n=1&ref=a',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_pages" VALUES('7','list.test','/items','n=2','https://list.test/items?n=2',X'3232323232323232323232323232323232323232323232323232323232323232');
INSERT INTO "rules_pages" VALUES('8','list.test','/items','n=2&ref=b','https://list.test/items?n=2&ref=b',X'3232323232323232323232323232323232323232323232323232323232323232');
INSERT INTO "rules_pages" VALUES('9','list.test','/items','n=3','https://list.test/items?n=3',X'3333333333333333333333333333333333333333333333333333333333333333');
CREATE TABLE rules_pairs (
            host TEXT NOT NULL,
            parameter TEXT NOT NULL,
            form TEXT NOT NULL,
            one TEXT NOT NULL,
            other TEXT NOT NULL,
            matching INTEGER NOT NULL,
            UNIQUE (host, parameter, form, one, other)
        ) STRICT;
INSERT INTO "rules_pairs" VALUES('list.test','ref','','https://list.test/items?n=1','https://list.test/items?n=1&ref=a',1);
INSERT INTO "rules_pairs" VALUES('list.test','n','','https://list.test/items?n=1','https://list.test/items?n=2',0);
INSERT INTO "rules_pairs" VALUES('list.test','ref','','https://list.test/items?n=2','https://list.test/items?n=2&ref=b',1);
INSERT INTO "rules_pairs" VALUES('list.test','ref','','https://list.test/items?n=3','https://list.test/items?n=3&ref=c',1);
INSERT INTO "rules_pairs" VALUES('list.test','','trailing_slash','https://list.test/guide','https://list.test/guide/',0);
INSERT INTO "rules_pairs" VALUES('list.test','offset','','https://list.test/rows?offset=0','https://list.test/rows?offset=20',0);
CREATE TABLE rules_patterns (
            host TEXT NOT NULL,
            pattern TEXT NOT NULL,
            streak INTEGER NOT NULL,
            first TEXT,
            PRIMARY KEY (host, pattern)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_patterns" VALUES('guarded.test','/',0,NULL);
INSERT INTO "rules_patterns" VALUES('list.test','/guide',0,NULL);
INSERT INTO "rules_patterns" VALUES('list.test','/guide/',0,NULL);
INSERT INTO "rules_patterns" VALUES('list.test','/items',1,'https://list.test/items?n=3&ref=c');
INSERT INTO "rules_patterns" VALUES('list.test','/rows',0,NULL);
INSERT INTO "rules_patterns" VALUES('outlet.test','/sale',0,NULL);
INSERT INTO "rules_patterns" VALUES('shop.test','/',0,NULL);
INSERT INTO "rules_patterns" VALUES('shop.test','/broken',0,NULL);
INSERT INTO "rules_patterns" VALUES('shop.test','/pages/{number}',0,NULL);
CREATE TABLE rules_residues (
            crawl_request TEXT NOT NULL,
            parameter TEXT NOT NULL,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            residue TEXT NOT NULL,
            url TEXT NOT NULL,
            signature BLOB NOT NULL,
            PRIMARY KEY (crawl_request, parameter)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_residues" VALUES('10','ref','list.test','/items','n=3','https://list.test/items?n=3&ref=c',X'3333333333333333333333333333333333333333333333333333333333333333');
INSERT INTO "rules_residues" VALUES('11','offset','list.test','/rows','','https://list.test/rows?offset=0',X'BD5CC37D2B90B36FDBB5AC71E8B9B54089950B1F205EC4B7E39C7FC2124ABA39');
INSERT INTO "rules_residues" VALUES('12','offset','list.test','/rows','','https://list.test/rows?offset=20',X'DA9F555019388929252426958FB762AD8F362704D5B726EAA8A8C71633CBD287');
INSERT INTO "rules_residues" VALUES('5','n','list.test','/items','','https://list.test/items?n=1',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_residues" VALUES('6','n','list.test','/items','ref=a','https://list.test/items?n=1&ref=a',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_residues" VALUES('6','ref','list.test','/items','n=1','https://list.test/items?n=1&ref=a',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_residues" VALUES('7','n','list.test','/items','','https://list.test/items?n=2',X'3232323232323232323232323232323232323232323232323232323232323232');
INSERT INTO "rules_residues" VALUES('8','ref','list.test','/items','n=2','https://list.test/items?n=2&ref=b',X'3232323232323232323232323232323232323232323232323232323232323232');
CREATE TABLE rules_signatures (
            host TEXT NOT NULL,
            signature BLOB NOT NULL,
            PRIMARY KEY (host, signature)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "rules_signatures" VALUES('guarded.test',X'0DB7F1157A3804BAAABB5F555AD30FF56B33C2863FE35359A8A869DA600F938F');
INSERT INTO "rules_signatures" VALUES('list.test',X'3131313131313131313131313131313131313131313131313131313131313131');
INSERT INTO "rules_signatures" VALUES('list.test',X'3232323232323232323232323232323232323232323232323232323232323232');
INSERT INTO "rules_signatures" VALUES('list.test',X'3333333333333333333333333333333333333333333333333333333333333333');
INSERT INTO "rules_signatures" VALUES('list.test',X'6767676767676767676767676767676767676767676767676767676767676767');
INSERT INTO "rules_signatures" VALUES('list.test',X'6868686868686868686868686868686868686868686868686868686868686868');
INSERT INTO "rules_signatures" VALUES('list.test',X'BD5CC37D2B90B36FDBB5AC71E8B9B54089950B1F205EC4B7E39C7FC2124ABA39');
INSERT INTO "rules_signatures" VALUES('list.test',X'C0824ED446F6456FF101EF50F00BE6A7D66B65D00E521AD697138B090D0FBB6F');
INSERT INTO "rules_signatures" VALUES('list.test',X'DA9F555019388929252426958FB762AD8F362704D5B726EAA8A8C71633CBD287');
INSERT INTO "rules_signatures" VALUES('outlet.test',X'12AFC63172B9490AD94BAD409108B6621E3F366FAC138B4617C6A18139BD5D0F');
INSERT INTO "rules_signatures" VALUES('shop.test',X'02EC18F1FB312DAD2BFDECB2306A105221BCD8686CD656D6F4D18BD81C07D991');
INSERT INTO "rules_signatures" VALUES('shop.test',X'4BFF38ABCDC6277E34168C3B3C0C5DD0B2B4E82EB9449D0DB143004BFCBAAAFD');
INSERT INTO "rules_signatures" VALUES('shop.test',X'E5ADAA0D5A50ACDD9DB6B0DFE44539278670AAF516E549CB75C7D7205EF87EC8');
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
        report TEXT,
        charges TEXT NOT NULL DEFAULT 'synced' CHECK (charges IN ('synced', 'deferred', 'unsynced'))
    ) STRICT;
INSERT INTO "run" VALUES('crawl','crawl','foxhound','os_user',1,'{"seeds":[{"label":"home","payload":{"depth":0},"url":"https://shop.test/"},{"label":"product","url":"https://shop.test/broken"},{"label":"home","url":"https://SHOP.test:443/"},{"body":"eyJwYWdlIjoxfQ==","key":"search-1","label":"home","method":"POST","url":"https://api.test/search"},{"label":"guarded","url":"https://guarded.test/"},{"label":"home","url":"https://shop.test/more/more/more/"},{"label":"listed","url":"https://list.test/items?n=1"},{"label":"listed","url":"https://list.test/items?n=1&ref=a"},{"label":"listed","url":"https://list.test/items?n=2"},{"label":"listed","url":"https://list.test/items?n=2&ref=b"},{"label":"listed","url":"https://list.test/items?n=3"},{"label":"listed","url":"https://list.test/items?n=3&ref=c"},{"label":"listed","url":"https://list.test/rows?offset=0"},{"label":"listed","url":"https://list.test/rows?offset=20"},{"label":"listed","url":"https://list.test/rows?offset=40"},{"label":"home","url":"https://shop.test/pages/1"},{"label":"home","url":"https://shop.test/pages/2"},{"label":"listed","url":"https://list.test/guide"},{"label":"listed","url":"https://list.test/guide/"}]}','finished',NULL,'verified','incomplete','2026-01-05T09:00:00Z','2026-01-05T09:00:04Z','sha256:0227dc879fda140d','[]',4,NULL,'sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9',0,NULL,NULL,'{"run_id":"crawl","lifecycle":"finished","verdict":"verified","completeness":"incomplete","completeness_reasons":[{"code":"crawl.max_segment_repeats","message":"`crawl.max_segment_repeats` refused 1 URL the crawl would otherwise have admitted.","count":1},{"code":"crawl.path_budgets","message":"`crawl.path_budgets`''s entry `/pages/*` refused 1 URL past its 1 request the crawl would otherwise have admitted.","count":1},{"code":"failed_requests","message":"12 requests in the crawl''s scope ended `failed` (`deadline_exceeded`: 12), so the pages only they link were never reached; `failedRequests()` lists them and `retryFailed()` re-admits them.","count":12}],"started_at":"2026-01-05T09:00:00Z","ended_at":"2026-01-05T09:00:04Z","usage":{"model":{"calls":0,"tokens":0,"input_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0,"output_tokens":0,"reasoning_tokens":0,"price":null},"browser_time":0,"proxy_bytes":0,"requests":17,"limits":[{"scope":{"kind":"install","id":""},"usage_kind":"requests","amount":1000,"left":983,"set_by":"client"}]},"policy_version":"sha256:0227dc879fda140d","learned_rules":[{"evidence":"3 pairs of URLs differing only by `ref` served pages of matching signatures, and no pair pages of differing ones: https://list.test/items?n=1 and https://list.test/items?n=1&ref=a; https://list.test/items?n=2 and https://list.test/items?n=2&ref=b; https://list.test/items?n=3 and https://list.test/items?n=3&ref=c.","host":"list.test","kind":"no_effect_parameter","merged":["https://list.test/items?n=3&ref=d","https://list.test/items?n=1&ref=z"],"parameter":"ref"},{"contradiction":"https://list.test/guide and https://list.test/guide/, both fetched and neither redirected, served pages of differing signatures.","evidence":"https://list.test/guide redirected to https://list.test/guide/, which differs from it only by a trailing slash.","form":"trailing_slash","host":"list.test","kind":"equivalent_form","merged":[]},{"evidence":"`offset` steps by 20 on https://list.test/rows (https://list.test/rows?offset=0, https://list.test/rows?offset=20, https://list.test/rows?offset=40), and three of its pages each link one value of `after` of their own: https://list.test/rows?offset=0 links https://list.test/rows?after=r20/sale, https://list.test/rows?offset=20 links https://list.test/rows?after=r40/sale, and https://list.test/rows?offset=40 links https://list.test/rows?after=r60/sale.","host":"list.test","kind":"cursor_pagination","merged":[],"parameter":"offset","path":"/rows"}],"records":[{"committed":4,"dataset":"links","dropped":0,"rejected":0},{"committed":4,"dataset":"pages","dropped":0,"rejected":1}],"requests":{"blocked":1,"by_block_cause":[{"count":1,"name":"ip_block"}],"by_error_code":[{"count":12,"name":"deadline_exceeded"}],"failed":12,"pending":0,"skipped":0,"succeeded":4},"sitemaps":[{"added":0,"failed":0,"failures":[],"not_added":2,"not_added_by_reason":[{"count":1,"name":"no_rule"},{"count":1,"name":"out_of_scope"}],"read":1,"seed":"https://maps.test/sitemap.xml"},{"added":0,"failed":1,"failures":[{"code":"response_not_ok","message":"The sitemap https://maps.test/missing.xml answered 404, not a sitemap.","status":404,"url":"https://maps.test/missing.xml"}],"not_added":0,"not_added_by_reason":[],"read":0,"seed":"https://maps.test/missing.xml"}]}','unsynced');
CREATE TABLE run_file_chunks (sha256 TEXT NOT NULL, offset INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (sha256, offset)) STRICT;
CREATE TABLE run_files (position INTEGER NOT NULL PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, sha256 TEXT NOT NULL, size INTEGER NOT NULL) STRICT;
CREATE TABLE run_key (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    wrapped BLOB NOT NULL,
    key_reference TEXT NOT NULL
) STRICT;
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
INSERT INTO "scope_seeds" VALUES('list.test',0,'https','list.test');
INSERT INTO "scope_seeds" VALUES('maps.test',0,'https','maps.test');
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
INSERT INTO "seeds" VALUES(7,'{"kind":"added","id":"5","url":"https://list.test/items?n=1"}');
INSERT INTO "seeds" VALUES(8,'{"kind":"added","id":"6","url":"https://list.test/items?n=1&ref=a"}');
INSERT INTO "seeds" VALUES(9,'{"kind":"added","id":"7","url":"https://list.test/items?n=2"}');
INSERT INTO "seeds" VALUES(10,'{"kind":"added","id":"8","url":"https://list.test/items?n=2&ref=b"}');
INSERT INTO "seeds" VALUES(11,'{"kind":"added","id":"9","url":"https://list.test/items?n=3"}');
INSERT INTO "seeds" VALUES(12,'{"kind":"added","id":"10","url":"https://list.test/items?n=3&ref=c"}');
INSERT INTO "seeds" VALUES(13,'{"kind":"added","id":"11","url":"https://list.test/rows?offset=0"}');
INSERT INTO "seeds" VALUES(14,'{"kind":"added","id":"12","url":"https://list.test/rows?offset=20"}');
INSERT INTO "seeds" VALUES(15,'{"kind":"added","id":"13","url":"https://list.test/rows?offset=40"}');
INSERT INTO "seeds" VALUES(16,'{"kind":"added","id":"14","url":"https://shop.test/pages/1"}');
INSERT INTO "seeds" VALUES(17,'{"kind":"not_added","reason":"path_budget","url":"https://shop.test/pages/2"}');
INSERT INTO "seeds" VALUES(18,'{"kind":"added","id":"15","url":"https://list.test/guide"}');
INSERT INTO "seeds" VALUES(19,'{"kind":"added","id":"16","url":"https://list.test/guide/"}');
CREATE TABLE sitemap_entries (
            work_item TEXT NOT NULL,
            reason TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (work_item, reason)
        ) STRICT, WITHOUT ROWID;
INSERT INTO "sitemap_entries" VALUES('21','no_rule',1);
INSERT INTO "sitemap_entries" VALUES('21','out_of_scope',1);
CREATE TABLE sitemap_failures (
            url TEXT NOT NULL PRIMARY KEY,
            seed TEXT NOT NULL,
            code TEXT NOT NULL,
            status INTEGER,
            message TEXT NOT NULL
        ) STRICT;
INSERT INTO "sitemap_failures" VALUES('https://maps.test/missing.xml','22','response_not_ok',404,'The sitemap https://maps.test/missing.xml answered 404, not a sitemap.');
CREATE TABLE sitemaps (
            work_item TEXT NOT NULL PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            seed TEXT NOT NULL,
            rules TEXT NOT NULL,
            read INTEGER NOT NULL,
            nesting INTEGER NOT NULL
        ) STRICT;
INSERT INTO "sitemaps" VALUES('21','https://maps.test/sitemap.xml','21','[{"pattern":"**/books/*","label":"home"}]',1,0);
INSERT INTO "sitemaps" VALUES('22','https://maps.test/missing.xml','22','[{"pattern":"**/books/*","label":"home"}]',0,0);
CREATE TABLE tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT;
INSERT INTO "tags" VALUES(0,'schedule:nightly');
CREATE TABLE tier_choices (
            label TEXT NOT NULL,
            domain_key TEXT NOT NULL,
            unit TEXT NOT NULL,
            PRIMARY KEY (label, domain_key)
        ) STRICT;
CREATE TABLE trap_link_suspected (
            work_item TEXT NOT NULL PRIMARY KEY,
            revealable INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE unclean (id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1)) STRICT;
CREATE TABLE url_fragments (
            scope TEXT NOT NULL PRIMARY KEY,
            keeps INTEGER NOT NULL
        ) STRICT;
INSERT INTO "url_fragments" VALUES('run',0);
CREATE TABLE usage (
        slice_id INTEGER NOT NULL,
        epoch INTEGER NOT NULL,
        used INTEGER NOT NULL,
        PRIMARY KEY (slice_id, epoch)
    ) STRICT;
INSERT INTO "usage" VALUES(1,1,10);
INSERT INTO "usage" VALUES(2,1,7);
CREATE TABLE usage_parts (
        usage_kind TEXT NOT NULL,
        part TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, part)
    ) STRICT;
CREATE TABLE usage_totals (
        usage_kind TEXT NOT NULL,
        currency TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, currency)
    ) STRICT;
INSERT INTO "usage_totals" VALUES('requests','',17);
CREATE TABLE visited (
            identity TEXT NOT NULL PRIMARY KEY,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "visited" VALUES('GET https://guarded.test/','4');
INSERT INTO "visited" VALUES('GET https://list.test/guide','15');
INSERT INTO "visited" VALUES('GET https://list.test/guide/','16');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=1','5');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=1&ref=a','6');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=2','7');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=2&ref=b','8');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=3','9');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=3&ref=c','10');
INSERT INTO "visited" VALUES('GET https://list.test/items?n=3&ref=d','17');
INSERT INTO "visited" VALUES('GET https://list.test/rows?after=r20/sale','18');
INSERT INTO "visited" VALUES('GET https://list.test/rows?after=r40/sale','19');
INSERT INTO "visited" VALUES('GET https://list.test/rows?after=r60/sale','20');
INSERT INTO "visited" VALUES('GET https://list.test/rows?offset=0','11');
INSERT INTO "visited" VALUES('GET https://list.test/rows?offset=20','12');
INSERT INTO "visited" VALUES('GET https://list.test/rows?offset=40','13');
INSERT INTO "visited" VALUES('GET https://outlet.test/sale','23');
INSERT INTO "visited" VALUES('GET https://shop.test/','1');
INSERT INTO "visited" VALUES('GET https://shop.test/broken','2');
INSERT INTO "visited" VALUES('GET https://shop.test/pages/1','14');
INSERT INTO "visited" VALUES('key:search-1','3');
CREATE TABLE warnings (
        position INTEGER NOT NULL PRIMARY KEY,
        code TEXT NOT NULL,
        message TEXT NOT NULL,
        sessions TEXT NOT NULL,
        folder TEXT
    ) STRICT;
CREATE TABLE work_item_positions (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            reserved INTEGER NOT NULL
        ) STRICT;
INSERT INTO "work_item_positions" VALUES(1,23);
CREATE TABLE work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            method TEXT NOT NULL,
            body BLOB,
            label TEXT NOT NULL,
            depth INTEGER NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            retried_from INTEGER NOT NULL,
            lease_epoch INTEGER NOT NULL,
            error_code TEXT,
            domain_key TEXT,
            priority INTEGER NOT NULL,
            forefront INTEGER
        ) STRICT;
INSERT INTO "work_items" VALUES('1',1,'https://shop.test/','GET',NULL,'home',0,'succeeded',1,0,1,NULL,'shop.test',0,NULL);
INSERT INTO "work_items" VALUES('2',2,'https://shop.test/broken','GET',NULL,'product',0,'failed',1,0,1,'deadline_exceeded','shop.test',0,NULL);
INSERT INTO "work_items" VALUES('3',3,'https://api.test/search','POST',X'7B2270616765223A317D','home',0,'succeeded',1,0,1,NULL,'api.test',0,NULL);
INSERT INTO "work_items" VALUES('4',4,'https://guarded.test/','GET',NULL,'guarded',0,'blocked',1,0,1,NULL,'guarded.test',0,NULL);
INSERT INTO "work_items" VALUES('5',5,'https://list.test/items?n=1','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('6',6,'https://list.test/items?n=1&ref=a','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('7',7,'https://list.test/items?n=2','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('8',8,'https://list.test/items?n=2&ref=b','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('9',9,'https://list.test/items?n=3','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('10',10,'https://list.test/items?n=3&ref=c','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('11',11,'https://list.test/rows?offset=0','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('12',12,'https://list.test/rows?offset=20','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('13',13,'https://list.test/rows?offset=40','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('14',14,'https://shop.test/pages/1','GET',NULL,'home',0,'succeeded',1,0,1,NULL,'shop.test',0,NULL);
INSERT INTO "work_items" VALUES('15',15,'https://list.test/guide','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('16',16,'https://list.test/guide/','GET',NULL,'listed',0,'failed',1,0,1,'deadline_exceeded','list.test',0,NULL);
INSERT INTO "work_items" VALUES('17',17,'https://list.test/items?n=3&ref=d','GET',NULL,'',0,'skipped',1,0,1,NULL,'list.test',0,NULL);
INSERT INTO "work_items" VALUES('18',18,'https://list.test/rows?after=r20/sale','GET',NULL,'',0,'succeeded',1,0,1,NULL,'list.test',0,NULL);
INSERT INTO "work_items" VALUES('19',19,'https://list.test/rows?after=r40/sale','GET',NULL,'',0,'succeeded',1,0,1,NULL,'list.test',0,NULL);
INSERT INTO "work_items" VALUES('20',20,'https://list.test/rows?after=r60/sale','GET',NULL,'',0,'succeeded',1,0,1,NULL,'list.test',0,NULL);
INSERT INTO "work_items" VALUES('21',21,'https://maps.test/sitemap.xml','GET',NULL,'',0,'succeeded',1,0,1,NULL,'maps.test',0,NULL);
INSERT INTO "work_items" VALUES('22',22,'https://maps.test/missing.xml','GET',NULL,'',0,'failed',1,0,1,NULL,'maps.test',0,NULL);
INSERT INTO "work_items" VALUES('23',23,'https://outlet.test/sale','GET',NULL,'home',1,'succeeded',1,0,1,NULL,'outlet.test',0,23);
CREATE INDEX outcomes_by_work_item ON outcomes (work_item, position);
CREATE INDEX pool_sessions_open ON pool_sessions (pool, position)
            WHERE retired_because IS NULL;
CREATE INDEX rules_ended_by_rule ON rules_ended (rule);
CREATE INDEX rules_pages_by_query ON rules_pages (host, path, query);
CREATE INDEX rules_pages_by_url ON rules_pages (url);
CREATE INDEX rules_residues_by_residue
            ON rules_residues (host, path, residue, parameter);
CREATE INDEX scope_seeds_by_domain ON scope_seeds (domain);
CREATE INDEX work_items_by_domain
            ON work_items (status, domain_key, forefront DESC, priority DESC, position);
CREATE INDEX work_items_by_status ON work_items (status, position);
CREATE INDEX work_items_unkeyed ON work_items (position)
            WHERE domain_key IS NULL;
COMMIT;
