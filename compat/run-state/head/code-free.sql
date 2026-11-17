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
            max_sitemap_nesting INTEGER,
            learning INTEGER NOT NULL
        ) STRICT;
INSERT INTO "bounds" VALUES(1,NULL,NULL,2083,20,2,5,1);
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
INSERT INTO "bounds_refusals" VALUES('crawl.max_segment_repeats',1);
CREATE TABLE code_free_extraction (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            extraction TEXT NOT NULL
        ) STRICT;
INSERT INTO "code_free_extraction" VALUES(1,'{"formats":{"markdown":true},"follow":"none"}');
CREATE TABLE datasets (
            name TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL,
            schema TEXT,
            schema_hash TEXT,
            key TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
INSERT INTO "datasets" VALUES('pages',0,'{"$defs":{"Contacts":{"additionalProperties":false,"description":"The contacts the page shows, each value once, where it first appears.","properties":{"emails":{"arrived_in":"0.1","description":"Its e-mail addresses, from `mailto:` links and from text matching\nthe HTML Standard''s valid e-mail address syntax.","items":{"$ref":"#/$defs/Email"},"type":"array"},"phones":{"arrived_in":"0.1","description":"Its telephone numbers, from `tel:` links and from text.","items":{"$ref":"#/$defs/Phone"},"type":"array"},"social":{"arrived_in":"0.1","description":"Its social profiles, from links to the extractor''s list of networks.","items":{"$ref":"#/$defs/SocialProfile"},"type":"array"}},"required":["emails","phones","social"],"type":"object"},"Email":{"additionalProperties":false,"description":"An e-mail address the page shows.","properties":{"text":{"arrived_in":"0.1","description":"The text it was read from: its `mailto:` link''s text, else the\naddress as the text writes it.","type":"string"},"value":{"arrived_in":"0.1","description":"The address.","type":"string"}},"required":["value","text"],"type":"object"},"Image":{"additionalProperties":false,"description":"An image of the page: an `<img src>`, in document order.","properties":{"alt":{"arrived_in":"0.1","description":"Its `alt`: empty for an image marked decorative, `null` when it has\nnone.","type":["string","null"]},"src":{"arrived_in":"0.1","description":"Its `src` as the document writes it.","type":"string"},"url":{"arrived_in":"0.1","description":"Its `src` resolved against the document''s base URL; `null` when it\ndoes not parse.","type":["string","null"]}},"required":["url","src","alt"],"type":"object"},"Link":{"additionalProperties":false,"description":"A link of the page: an `<a href>` or `<area href>`, in document order.","properties":{"href":{"arrived_in":"0.1","description":"Its `href` as the document writes it, its character references\ndecoded.","type":"string"},"rel":{"arrived_in":"0.1","description":"The tokens of its `rel`, as written; empty when it has none.","items":{"type":"string"},"type":"array"},"text":{"arrived_in":"0.1","description":"Its text, white space collapsed.","type":"string"},"url":{"arrived_in":"0.1","description":"Its `href` resolved against the document''s base URL by the WHATWG\nURL Standard; `null` when it does not parse.","type":["string","null"]}},"required":["url","href","text","rel"],"type":"object"},"MainContent":{"description":"Whether the extractor found the document''s main content.","oneOf":[{"const":"found","description":"It found it.","type":"string"},{"const":"none","description":"The document has none: the formats read from it are empty, a value\nand never the whole page in its place.","type":"string"}]},"Media":{"additionalProperties":false,"description":"A media source of the page: a `<video src>`, an `<audio src>` or a\n`<source src>` of either, in document order.","properties":{"kind":{"$ref":"#/$defs/MediaKind","arrived_in":"0.1","description":"Whether it plays video or audio."},"src":{"arrived_in":"0.1","description":"Its `src` as the document writes it.","type":"string"},"type":{"arrived_in":"0.1","description":"Its `type`, the MIME type a `<source>` declares; `null` when it has\nnone.","type":["string","null"]},"url":{"arrived_in":"0.1","description":"Its `src` resolved against the document''s base URL; `null` when it\ndoes not parse.","type":["string","null"]}},"required":["url","src","kind","type"],"type":"object"},"MediaKind":{"description":"Whether a media source plays video or audio.","oneOf":[{"const":"video","description":"A `<video>`''s, its own `src` or one of its `<source>`s.","type":"string"},{"const":"audio","description":"An `<audio>`''s, its own `src` or one of its `<source>`s.","type":"string"}]},"MetaTag":{"additionalProperties":false,"description":"One `<meta>` tag of a vocabulary, Open Graph''s or Twitter''s, in\ndocument order: a property a page writes more than once, such as\n`og:image`, is listed once per tag.","properties":{"content":{"arrived_in":"0.1","description":"Its `content`.","type":"string"},"property":{"arrived_in":"0.1","description":"Its `property`, else its `name`, as written, such as `og:title`.","type":"string"}},"required":["property","content"],"type":"object"},"Metadata":{"additionalProperties":false,"description":"The page''s metadata, each value as the document writes it.","properties":{"canonical":{"arrived_in":"0.1","description":"Its `<link rel=\"canonical\">`''s `href` resolved against the base URL;\n`null` when it has none or it does not parse.","type":["string","null"]},"description":{"arrived_in":"0.1","description":"Its `<meta name=\"description\">`''s content; `null` when it has none.","type":["string","null"]},"json_ld":{"arrived_in":"0.1","description":"Each `<script type=\"application/ld+json\">` block that parses, as\nJSON, in document order.","items":true,"type":"array"},"lang":{"arrived_in":"0.1","description":"Its root element''s `lang`; `null` when it has none.","type":["string","null"]},"meta":{"additionalProperties":{"type":"string"},"arrived_in":"0.1","description":"Its `<meta name>` tags other than Open Graph''s and Twitter''s, by name\nin lower case, the first of a name kept.","type":"object"},"microdata":{"arrived_in":"0.1","description":"Its top-level microdata items, in tree order.","items":{"$ref":"#/$defs/MicrodataItem"},"type":"array"},"open_graph":{"arrived_in":"0.1","description":"Its Open Graph tags: every `<meta property>` not named `twitter:`,\nand every `<meta name>` named `og:`.","items":{"$ref":"#/$defs/MetaTag"},"type":"array"},"title":{"arrived_in":"0.1","description":"Its first `<title>`''s text, white space collapsed; `null` when it\nhas none.","type":["string","null"]},"twitter":{"arrived_in":"0.1","description":"Its Twitter tags, those named `twitter:`.","items":{"$ref":"#/$defs/MetaTag"},"type":"array"}},"required":["title","description","canonical","lang","open_graph","twitter","json_ld","microdata","meta"],"type":"object"},"MicrodataItem":{"additionalProperties":false,"description":"A microdata item (the HTML Standard''s microdata), as its JSON\nconversion writes it, with every field given.","properties":{"id":{"arrived_in":"0.1","description":"Its `itemid`, resolved against the document''s base URL; `null` when\nit has none or it does not parse.","type":["string","null"]},"properties":{"additionalProperties":{"items":{"$ref":"#/$defs/MicrodataValue"},"type":"array"},"arrived_in":"0.1","description":"Its properties, by name, each name''s values in tree order.","type":"object"},"type":{"arrived_in":"0.1","description":"Its `itemtype` tokens; empty when it has none.","items":{"type":"string"},"type":"array"}},"required":["type","id","properties"],"type":"object"},"MicrodataValue":{"anyOf":[{"description":"Text: the element''s content, value or resolved URL, as the HTML\nStandard reads each element.","type":"string"},{"$ref":"#/$defs/MicrodataItem","description":"An item, the element having `itemscope`."}],"description":"The value of a microdata property: text, or an item of its own."},"Phone":{"additionalProperties":false,"description":"A telephone number the page shows.","properties":{"e164":{"arrived_in":"0.1","description":"The number in E.164, such as `+33123456789`, when it carries its\ncountry code or the document''s language tag carries a region, and\nit is a valid number there; `null` otherwise, as no country is\nguessed.","type":["string","null"]},"text":{"arrived_in":"0.1","description":"The text it was read from: its `tel:` link''s text, else the number\nas the text writes it.","type":"string"},"value":{"arrived_in":"0.1","description":"The number as written: its `tel:` link''s number, else the text''s.","type":"string"}},"required":["value","text","e164"],"type":"object"},"SocialNetwork":{"description":"A social network whose profiles the page links to.","oneOf":[{"const":"x","description":"X, formerly Twitter.","type":"string"},{"const":"linkedin","description":"`LinkedIn`.","type":"string"},{"const":"facebook","description":"Facebook.","type":"string"},{"const":"instagram","description":"Instagram.","type":"string"},{"const":"youtube","description":"`YouTube`.","type":"string"},{"const":"tiktok","description":"`TikTok`.","type":"string"},{"const":"github","description":"`GitHub`.","type":"string"},{"const":"mastodon","description":"A Mastodon instance''s profile, linked with `rel=\"me\"`.","type":"string"}]},"SocialProfile":{"additionalProperties":false,"description":"A social profile the page links to.","properties":{"network":{"$ref":"#/$defs/SocialNetwork","arrived_in":"0.1","description":"Its network."},"text":{"arrived_in":"0.1","description":"Its link''s text, white space collapsed.","type":"string"},"value":{"arrived_in":"0.1","description":"The profile''s URL, resolved against the document''s base URL.","type":"string"}},"required":["value","text","network"],"type":"object"},"Table":{"additionalProperties":false,"description":"A table of the page, in document order, a table inside another listed\non its own: each row its cells'' text, white space collapsed, a row''s\nheader kept as its first cell.","properties":{"body_rows":{"arrived_in":"0.1","description":"Its other rows, its `<tfoot>`''s last.","items":{"items":{"type":"string"},"type":"array"},"type":"array"},"caption":{"arrived_in":"0.1","description":"Its `<caption>`''s text; `null` when it has none.","type":["string","null"]},"header_rows":{"arrived_in":"0.1","description":"Its header rows: its `<thead>`''s, else the rows it begins with whose\ncells are all `<th>`.","items":{"items":{"type":"string"},"type":"array"},"type":"array"}},"required":["caption","header_rows","body_rows"],"type":"object"}},"$schema":"https://json-schema.org/draft/2020-12/schema","additionalProperties":false,"description":"What `page.formats` answers: the formats the request asked for, and\nnone it did not, with the document''s URL, its base URL and, when a\nformat it asked for is read from the main content, whether the document\nhas main content. Text is as the document holds it, never repaired.","properties":{"base_url":{"arrived_in":"0.1","description":"The URL its relative URLs resolve against: its first `<base href>`,\nelse its URL.","type":"string"},"fetched_at":{"description":"When the engine read the page, in UTC (RFC 3339).","format":"date-time","type":"string"},"main_content":{"$ref":"#/$defs/MainContent","arrived_in":"0.1","description":"Whether it has main content, when the request asked for a format\nread from it ([`FormatsRequest::reads_main_content`])."},"markdown":{"arrived_in":"0.1","description":"Its content as Markdown, when the request asked for it.","type":"string"},"status":{"description":"The status of the page''s response; null when the browser''s navigation answered none.","maximum":999,"minimum":100,"type":["integer","null"]},"url":{"arrived_in":"0.1","description":"The document''s URL, after redirects.","type":"string"}},"required":["url","base_url","main_content","markdown","status","fetched_at"],"title":"PageFormats","type":"object"}','29e39ea142888563aa1625f0c44c4072d0eb8ed000b56bb80ea1f5829ed437c6','["url"]');
CREATE TABLE domains (
            key TEXT NOT NULL PRIMARY KEY,
            ready_at INTEGER,
            backoffs INTEGER NOT NULL
        ) STRICT;
CREATE TABLE effect_journal (entry INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, key TEXT NOT NULL, repeatable INTEGER NOT NULL, state TEXT NOT NULL CHECK (state IN ('open', 'sent', 'done', 'uncertain', 'reconciled')), output TEXT, reason TEXT, first_command TEXT, since TEXT, performed_at TEXT, reconciled_by TEXT, reconciled_at TEXT, UNIQUE (namespace, key), CHECK (state NOT IN ('sent', 'uncertain') OR since IS NOT NULL), CHECK (state <> 'done' OR performed_at IS NOT NULL), CHECK (state <> 'uncertain' OR reason IS NOT NULL), CHECK (state <> 'reconciled' OR (reconciled_by IS NOT NULL AND reconciled_at IS NOT NULL))) STRICT;
CREATE TABLE effect_secret (id INTEGER PRIMARY KEY CHECK (id = 1), secret BLOB NOT NULL) STRICT;
CREATE TABLE format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT;
INSERT INTO "format" VALUES('run-state','0.1');
CREATE TABLE key_values (key TEXT NOT NULL PRIMARY KEY, value TEXT, content BLOB, content_type TEXT, file INTEGER, CHECK ((value IS NULL) <> (content_type IS NULL)), CHECK (content_type IS NULL OR ((content IS NULL) <> (file IS NULL)))) STRICT;
CREATE TABLE labels (
            name TEXT NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT;
INSERT INTO "labels" VALUES('page','{}');
CREATE TABLE learned_identities (
            identity TEXT NOT NULL PRIMARY KEY,
            rule INTEGER NOT NULL,
            crawl_request TEXT NOT NULL
        ) STRICT, WITHOUT ROWID;
CREATE TABLE learned_merges (
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            url TEXT NOT NULL,
            identity TEXT,
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
INSERT INTO "payload_labels" VALUES('page',NULL,NULL);
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
            sends_from TEXT NOT NULL,
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
            second_value TEXT,
            second_page TEXT,
            second_url TEXT,
            refuted INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID;
CREATE TABLE rules_ended (
            crawl_request TEXT NOT NULL,
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            PRIMARY KEY (crawl_request, rule)
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
        charges TEXT NOT NULL DEFAULT 'synced' CHECK (charges IN ('synced', 'deferred', 'unsynced'))
    ) STRICT;
INSERT INTO "run" VALUES('code-free','crawl','foxhound','os_user',1,'{"seeds":[{"label":"page","url":"https://shop.test/more/more/more/"}]}','finished',NULL,'verified','unverified','2026-01-05T09:00:04Z','2026-01-05T09:00:04Z','sha256:0227dc879fda140d','[]',2,NULL,'sha256:3c421046adfa879cb53dcf6f36eefe65204f24f04758122788a21c10537dc6b9',0,NULL,NULL,'{"run_id":"code-free","lifecycle":"finished","verdict":"verified","completeness":"unverified","completeness_reasons":[{"code":"crawl.max_segment_repeats","message":"`crawl.max_segment_repeats` refused 1 URL the crawl would otherwise have admitted.","count":1}],"started_at":"2026-01-05T09:00:04Z","ended_at":"2026-01-05T09:00:04Z","usage":{"model":{"calls":0,"tokens":0,"input_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0,"output_tokens":0,"reasoning_tokens":0,"price":null},"browser_time":0,"proxy_bytes":0,"requests":0,"limits":[{"scope":{"kind":"install","id":""},"usage_kind":"requests","amount":1000,"left":983,"set_by":"client"}]},"policy_version":"sha256:0227dc879fda140d","nothing_done":{"reason":"no_seed_handled","seeds":[{"reason":"max_segment_repeats","count":1}]},"learned_rules":[],"records":[],"requests":{"blocked":0,"failed":0,"pending":0,"skipped":0,"succeeded":0}}','unsynced');
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
INSERT INTO "scope" VALUES(1,'same_hostname','[]','[]');
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
INSERT INTO "seeds" VALUES(1,'{"kind":"not_added","reason":"max_segment_repeats","url":"https://shop.test/more/more/more/"}');
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
            read INTEGER NOT NULL,
            nesting INTEGER NOT NULL
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
