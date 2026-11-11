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
            max_segment_repeats INTEGER,
            learning INTEGER NOT NULL
        ) STRICT;
CREATE TABLE bounds_admitted (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            count INTEGER NOT NULL
        ) STRICT;
CREATE TABLE bounds_path_budgets (
            position INTEGER NOT NULL PRIMARY KEY,
            pattern TEXT NOT NULL,
            max_requests INTEGER NOT NULL
        ) STRICT;
CREATE TABLE bounds_path_counts (
            pattern TEXT NOT NULL PRIMARY KEY,
            admitted INTEGER NOT NULL,
            refused INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE bounds_refusals (
            setting TEXT NOT NULL PRIMARY KEY,
            count INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID;
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
CREATE TABLE domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT, content BLOB, content_type TEXT, file INTEGER, CHECK ((value IS NULL) <> (content_type IS NULL)), CHECK (content_type IS NULL OR ((content IS NULL) <> (file IS NULL)))) STRICT;
CREATE TABLE labels (
            name TEXT NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT;
CREATE TABLE learned_identities (
            identity TEXT NOT NULL PRIMARY KEY,
            rule INTEGER NOT NULL,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE learned_merges (
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            url TEXT NOT NULL,
            UNIQUE (rule, url)
        ) STRICT;
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
CREATE TABLE outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT;
CREATE TABLE page_sessions (position INTEGER NOT NULL PRIMARY KEY, is_default INTEGER NOT NULL, closed INTEGER NOT NULL, settings TEXT NOT NULL, identity_seed INTEGER NOT NULL, source TEXT) STRICT;
CREATE TABLE payload_cap (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            max_size INTEGER NOT NULL
        ) STRICT;
CREATE TABLE payload_labels (
            label TEXT NOT NULL PRIMARY KEY,
            schema TEXT,
            hash TEXT
        ) STRICT, WITHOUT ROWID;
CREATE TABLE payloads (
            request TEXT NOT NULL PRIMARY KEY,
            payload TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE pool_blocked_address_blocks (
            domain_key TEXT NOT NULL,
            block TEXT NOT NULL,
            PRIMARY KEY (domain_key, block)
        ) STRICT;
CREATE TABLE pool_sessions (
            pool TEXT NOT NULL,
            position INTEGER NOT NULL,
            proxy_tier TEXT NOT NULL,
            opened_at INTEGER NOT NULL,
            uses INTEGER NOT NULL,
            error_score REAL NOT NULL,
            retired_because TEXT,
            address_ordinal INTEGER,
            PRIMARY KEY (pool, position)
        ) STRICT;
CREATE TABLE pool_tiers (
            domain_key TEXT NOT NULL PRIMARY KEY,
            proxy_tier TEXT NOT NULL,
            browser INTEGER NOT NULL
        ) STRICT;
CREATE TABLE record_counts (
            dataset TEXT NOT NULL PRIMARY KEY,
            committed INTEGER NOT NULL,
            dropped INTEGER NOT NULL,
            rejected INTEGER NOT NULL
        ) STRICT;
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
CREATE TABLE rejected_records (
            dataset TEXT NOT NULL,
            number INTEGER NOT NULL,
            work_item TEXT NOT NULL,
            data TEXT NOT NULL,
            rejection TEXT NOT NULL,
            PRIMARY KEY (dataset, number)
        ) STRICT;
CREATE TABLE request_counts (
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (kind, name)
        ) STRICT;
CREATE TABLE robots_txt_copies (location TEXT NOT NULL PRIMARY KEY, status INTEGER, body BLOB, unreachable TEXT, CHECK ((status IS NULL) = (body IS NULL)), CHECK ((status IS NULL) <> (unreachable IS NULL))) STRICT;
CREATE TABLE rules_canonicals (
            host TEXT NOT NULL,
            form TEXT NOT NULL,
            url TEXT NOT NULL,
            canonical TEXT NOT NULL,
            PRIMARY KEY (host, form, url)
        ) STRICT, WITHOUT ROWID;
CREATE TABLE rules_cursors (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value TEXT NOT NULL,
            page TEXT NOT NULL,
            url TEXT NOT NULL,
            refuted INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID;
CREATE TABLE rules_offset_values (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value INTEGER NOT NULL,
            url TEXT NOT NULL,
            PRIMARY KEY (host, path, parameter, value)
        ) STRICT, WITHOUT ROWID;
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
CREATE TABLE rules_pages (
            crawl_request TEXT NOT NULL PRIMARY KEY,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            query TEXT NOT NULL,
            url TEXT NOT NULL,
            signature BLOB NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE rules_pairs (
            host TEXT NOT NULL,
            parameter TEXT NOT NULL,
            form TEXT NOT NULL,
            one TEXT NOT NULL,
            other TEXT NOT NULL,
            matching INTEGER NOT NULL,
            UNIQUE (host, parameter, form, one, other)
        ) STRICT;
CREATE TABLE rules_patterns (
            host TEXT NOT NULL,
            pattern TEXT NOT NULL,
            streak INTEGER NOT NULL,
            first TEXT,
            PRIMARY KEY (host, pattern)
        ) STRICT, WITHOUT ROWID;
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
CREATE TABLE rules_signatures (
            host TEXT NOT NULL,
            signature BLOB NOT NULL,
            PRIMARY KEY (host, signature)
        ) STRICT, WITHOUT ROWID;
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
        deferred_charges INTEGER NOT NULL DEFAULT 0
    ) STRICT;
INSERT INTO "run" VALUES('unclean','collie','collie','os_user',1,'{}','stopped','{"kind":"abort","by":"client"}','verified','incomplete','2026-01-05T09:00:00Z','2026-01-05T09:00:00Z','sha256:0227dc879fda140d','[{"key":"run_state.durability","value":"off","layer":"run","origin":"settings"}]',2,NULL,'sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9',0,NULL,NULL,'{"run_id":"unclean","lifecycle":"stopped","stop_reason":{"kind":"abort","by":"client"},"verdict":"verified","completeness":"incomplete","completeness_reasons":[{"code":"stopped","message":"It stopped early: aborted by its Client."}],"started_at":"2026-01-05T09:00:00Z","ended_at":"2026-01-05T09:00:00Z","usage":{"model":{"calls":0,"tokens":0},"browser_time":0,"proxy_bytes":0,"requests":0,"limits":[{"scope":{"kind":"install","id":""},"usage_kind":"requests","amount":1000,"left":983,"set_by":"client"}]},"weakened":[{"key":"run_state.durability","value":"off","layer":"run","origin":"settings"}],"policy_version":"sha256:0227dc879fda140d"}',0);
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
CREATE TABLE sitemap_entries (
            work_item TEXT NOT NULL,
            reason TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (work_item, reason)
        ) STRICT, WITHOUT ROWID;
CREATE TABLE sitemap_failures (
            url TEXT NOT NULL PRIMARY KEY,
            seed TEXT NOT NULL,
            code TEXT NOT NULL,
            status INTEGER,
            message TEXT NOT NULL
        ) STRICT;
CREATE TABLE sitemaps (
            work_item TEXT NOT NULL PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            seed TEXT NOT NULL,
            rules TEXT NOT NULL,
            read INTEGER NOT NULL
        ) STRICT;
CREATE TABLE tags (position INTEGER NOT NULL PRIMARY KEY, tag TEXT NOT NULL) STRICT;
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
INSERT INTO "unclean" VALUES(1);
CREATE TABLE url_fragments (
            scope TEXT NOT NULL PRIMARY KEY,
            keeps INTEGER NOT NULL
        ) STRICT;
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
CREATE TABLE usage_totals (
        usage_kind TEXT NOT NULL,
        currency TEXT NOT NULL,
        amount INTEGER NOT NULL,
        PRIMARY KEY (usage_kind, currency)
    ) STRICT;
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
CREATE TABLE work_item_positions (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            reserved INTEGER NOT NULL
        ) STRICT;
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
CREATE INDEX outcomes_by_work_item ON outcomes (work_item, position);
CREATE INDEX pool_sessions_open ON pool_sessions (pool, position)
            WHERE retired_because IS NULL;
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
