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
CREATE TABLE bounds (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_depth INTEGER,
            max_requests INTEGER,
            max_url_length INTEGER,
            max_path_segments INTEGER,
            max_segment_repeats INTEGER
        ) STRICT;
CREATE TABLE bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT;
CREATE TABLE bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT, content BLOB, content_type TEXT, file INTEGER, CHECK ((value IS NULL) <> (content_type IS NULL)), CHECK (content_type IS NULL OR ((content IS NULL) <> (file IS NULL)))) STRICT;
CREATE TABLE outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT;
CREATE TABLE page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL) STRICT;
INSERT INTO "page_sessions" VALUES(1,1,0,'{}',6178731391398722387);
INSERT INTO "page_sessions" VALUES(2,0,1,'{"filter_lists.ads":false,"proxy.server":"http://proxy.example:3128","proxy.username":"user","proxy.password":"env:PROXY_PASSWORD"}',-2937466112876520115);
CREATE TABLE pool_sessions (
            position INTEGER NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT
        ) STRICT;
CREATE TABLE pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL
        ) STRICT;
CREATE TABLE records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
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
INSERT INTO "run" VALUES('paused','collie','collie','os_user',1,'{}','stopped','{"kind":"pause"}','verified','incomplete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z',NULL,'[]',10,'{"kind":"browser_host","browser_host":"laptop"}','6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b',0,'Waiting for the next page','{"reason":"nothing_pending"}','{"run_id":"paused","lifecycle":"stopped","stop_reason":{"kind":"pause"},"verdict":"verified","completeness":"incomplete","completeness_reasons":[{"code":"stopped","message":"It stopped early: paused."}],"started_at":"2026-01-05T09:00:00Z","ended_at":"2026-01-05T09:00:00Z","usage":{"model":{"calls":0,"tokens":0},"browser_time":0,"proxy_bytes":4096,"proxy_bytes_by_kind":[{"kind":"datacenter","bytes":4096}],"requests":0,"limits":[{"scope":{"kind":"install","id":""},"usage_kind":"requests","amount":1000,"left":995,"set_by":"client"}]},"warnings":[{"code":"clean_up_rules_fallback","message":"The Run''s Clean-up rules are not the version asked for: the Run''s recorded Clean-up rules, 6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b, is gone from this machine: it holds to the embedded part, sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9, instead."},{"code":"storage_low","message":"The run folder''s filesystem met the `low` storage level.","sessions":["laptop"],"folder":"runs/paused"}],"nothing_done":{"reason":"nothing_pending"}}');
CREATE TABLE run_file_chunks (sha256 TEXT NOT NULL, offset INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (sha256, offset)) STRICT;
CREATE TABLE run_files (position INTEGER NOT NULL PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, sha256 TEXT NOT NULL, size INTEGER NOT NULL) STRICT;
INSERT INTO "run_files" VALUES(1,'download','books.csv','a5413d351b5da27721fae2446832bdbb181cf8c62f5131ba4101d4f47bcb8f56',22);
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
CREATE TABLE scope_seeds (
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            scheme TEXT NOT NULL,
            domain TEXT NOT NULL,
            PRIMARY KEY (host, port, scheme)
        ) STRICT, WITHOUT ROWID;
CREATE TABLE seeds (
            position INTEGER NOT NULL PRIMARY KEY,
            answer TEXT NOT NULL
        ) STRICT;
CREATE TABLE tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT;
CREATE TABLE usage (
        slice_id INTEGER NOT NULL,
        epoch INTEGER NOT NULL,
        used INTEGER NOT NULL,
        PRIMARY KEY (slice_id, epoch)
    ) STRICT;
CREATE TABLE usage_parts (
        usage_kind TEXT NOT NULL,
        part TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, part)
    ) STRICT;
INSERT INTO "usage_parts" VALUES('proxy_bytes','datacenter',4096);
CREATE TABLE usage_totals (
        usage_kind TEXT NOT NULL,
        currency TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, currency)
    ) STRICT;
INSERT INTO "usage_totals" VALUES('proxy_bytes','',4096);
CREATE TABLE visited (
            identity TEXT NOT NULL PRIMARY KEY,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE warnings (
        position INTEGER NOT NULL PRIMARY KEY,
        code TEXT NOT NULL,
        message TEXT NOT NULL,
        sessions TEXT NOT NULL,
        folder TEXT
    ) STRICT;
INSERT INTO "warnings" VALUES(1,'clean_up_rules_fallback','The Run''s Clean-up rules are not the version asked for: the Run''s recorded Clean-up rules, 6c8abc7a55f570dbb19512bdf4a9321337d1cb2c9b507c105b01b87fabbcc34b, is gone from this machine: it holds to the embedded part, sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9, instead.','[]',NULL);
INSERT INTO "warnings" VALUES(2,'storage_low','The run folder''s filesystem met the `low` storage level.','["laptop"]','runs/paused');
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
