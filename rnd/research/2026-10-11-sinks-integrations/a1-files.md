# A1. File, object-store and SaaS sinks: write primitives, Rust crates, CI doubles

Research date: 2026-10-11. Primary sources only. Crate data comes from the crates.io JSON API, fetched 2026-10-11. **Inference** marks anything not stated by a primary source.

Classes:
- **exactly-once fenced**: the destination can compare-and-set (CAS) on a value that carries the epoch, so a stale epoch is rejected server-side.
- **idempotent keyed**: a deterministic key, create-if-absent or upsert makes a retry harmless. A zombie with a lower epoch is not rejected; it writes under its own key.
- **at-least-once**: no server-side dedup primitive. Retries can duplicate.

"CAS on version" (ETag, generation, rev) gives fencing through a fence or manifest object. The pattern is: read the fence (epoch E, version V), check that E <= my epoch, then write with `If-Match: V`. A zombie either sees the higher epoch or loses the CAS. Data objects use create-if-absent under `delivery_id/epoch` keys. **Inference**: this is the standard optimistic-concurrency pattern and is not documented by any one vendor. Azure (`x-ms-if-tags`) and Box (JSON-Patch `test`) can compare the epoch value itself.

## Summary table

| Destination | Strongest primitive | Class | Rust crate (version, date, license, official?) | CI double | First-party in |
|---|---|---|---|---|---|
| AWS S3 | `If-None-Match: *` and `If-Match: <ETag>` on PutObject, CompleteMultipartUpload and CopyObject. 412 on failure. A bucket policy can require them. | exactly-once fenced (via fence object) | `aws-sdk-s3` 1.152.0, 2026-10-01, Apache-2.0, official. `object_store` 0.14.2, 2026-09-15, MIT/Apache-2.0, Apache Arrow project. `opendal` 0.59.4, 2026-10-05, Apache-2.0, Apache project | LocalStack (handles both headers in PutObject source). MinIO (repo archived). RustFS. testcontainers-modules 0.15.0 has `localstack`, `minio`, `rustfs` | Scrapy (`s3`), Zyte (Scrapy), Bright Data, Spider, Browse AI, Octoparse, Airbyte (`destination-s3`), dlt (filesystem) |
| Cloudflare R2 | PutObject: If-Match, If-None-Match, If-Modified-Since, If-Unmodified-Since. CompleteMultipartUpload: no conditional headers listed. CopyObject: `cf-copy-destination-if-*` (beta). | exactly-once fenced (single-part only) | Same S3 crates. `object_store` names R2 under `S3ConditionalPut::ETagMatch` | No official S3-API emulator found. Use an S3 double (**inference**) | Airbyte (`destination-r2`). dlt (S3 endpoint) |
| MinIO | If-Match and If-None-Match on PUT (MinIO blog; merged Feb 2023). | exactly-once fenced | Same S3 crates. `minio` 0.4.0, 2026-04-23, Apache-2.0, official | MinIO image. `minio/minio` repo archived on GitHub; last release RELEASE.2025-10-15 | dlt (S3-compatible) |
| Google Cloud Storage | `ifGenerationMatch` (JSON) or `x-goog-if-generation-match` (XML). `0` means "must not exist". Also metageneration. 412 on failure. XML multipart: preconditions not supported (400). | exactly-once fenced | `google-cloud-storage` 1.20.0, 2026-09-30, Apache-2.0, official (googleapis). `object_store`, `opendal` (gcs: `write_with_if_not_exists`, `write_with_if_version_match`) | fake-gcs-server v1.56.1 (2026-08-24) parses generation preconditions on upload | Scrapy (`gs`), Zyte, Bright Data, Spider, Airbyte (`destination-gcs`), dlt |
| Azure Blob | If-Match, If-None-Match (`*`), date conditions on Put Blob and Put Block List. 412 on failure. `x-ms-if-tags` takes a SQL-like predicate with `<`, `<=`, `=` on blob tags. | exactly-once fenced (can compare an epoch tag directly) | `azure_storage_blob` 1.1.0, 2026-09-08, MIT, official (Azure SDK for Rust). Upload options include `if_match`, `if_none_match`, `if_tags`. `object_store`, `opendal` (azblob: if_match, if_none_match, if_not_exists) | Azurite v3.37.0 (2026-08-26). Its write-condition validator handles `ifMatch` and `ifTags`. testcontainers `azurite` | Bright Data, Spider, Airbyte, dlt, Scrapy plugin (`scrapy-feedexporter-azure-storage`) |
| Backblaze B2 (S3 API) | None documented. The PutObject page lists no If-* headers. Proxmox reports B2 returns an error (501) on `If-None-Match`. | idempotent keyed (delivery_id + epoch in key) | S3 crates. `opendal` b2 service declares no conditional-write capability | MinIO or another S3 double (no B2 emulator found) | None found |
| Wasabi | No documentation found on conditional writes. | idempotent keyed (**inference**: treat as unsupported until tested) | S3 crates | S3 double | None found |
| Tigris | If-Match (CAS), If-None-Match `"*"`, If-Unmodified-Since on PutObject. 412 on failure. Use Single-region or Multi-region buckets for cross-region CAS. | exactly-once fenced | S3 crates | None found. Use an S3 double | dlt (S3-compatible) |
| Ceph RGW | Source (`rgw_rest_s3.cc`, main) reads `HTTP_IF_MATCH` and `HTTP_IF_NONE_MATCH` in PutObject and CompleteMultipart `get_params`. The docs feature table does not mention it. | exactly-once fenced (**inference** from source; test per release) | S3 crates | `quay.io/ceph` images or MicroCeph (**inference**; not checked) | None found |
| DigitalOcean Spaces | The S3 compatibility page does not mention conditional headers. | idempotent keyed (**inference**: unsupported until tested) | S3 crates | S3 double | None found |
| SFTP | `SSH_FXF_EXCL` (create fails if the file exists). Standard `SSH_FXP_RENAME` fails if the target exists. `posix-rename@openssh.com` is an atomic overwrite. `hardlink@openssh.com` is link(2): an atomic publish that fails if the target exists. | idempotent keyed (no CAS on content) | `openssh-sftp-client` 0.15.10, 2026-10-01, MIT, community: `create_new`, `rename` (uses posix-rename when present), `hard_link`. `russh-sftp` 3.0.1, 2026-09-28, Apache-2.0, community: `OpenFlags::EXCLUDE`, `hardlink`, no posix-rename helper. `ssh2` 0.9.6, 2026-06-30, MIT/Apache-2.0, community (libssh2, blocking): `OpenFlags::EXCLUSIVE`. `opendal` sftp: `write_with_if_not_exists` via `create_new` | `atmoz/sftp` (OpenSSH) as a generic container (**inference**). testcontainers-modules has no sftp module | Bright Data, Airbyte (`destination-sftp-json`), dlt, Zyte guide and Scrapy plugin (`scrapy-feedexporter-sftp`) |
| FTP/FTPS | RNFR/RNTO (overwrite behavior is server-defined). STOU picks a server-generated unique name, so it is not deterministic. No exclusive create. | idempotent keyed (best effort: temp upload, then rename to a deterministic name) | `suppaftp` 12.1.2, 2026-10-10, MIT/Apache-2.0, community: `rename`, `append_file`, `custom_command` (no STOU helper). `opendal` ftp builds on suppaftp | `delfer/alpine-ftp-server` (vsftpd), used by suppaftp's own testcontainers tests | Scrapy (`ftp`, `ftps` built in), Bright Data (SFTP or FTP) |
| Google Drive v3 | No precondition on `files.update`. The discovery doc has no ETag field on File. Create with a pre-generated ID (`files.generateIds`): retries return 409 and no duplicate is created. Not available for Workspace-native files. | idempotent keyed (create only). Updates are at-least-once | `google-drive3` 7.0.0+20251218, 2026-01-01, MIT, community (Byron/google-apis-rs). `opendal` gdrive (no conditional capability) | None | Apify, Octoparse, Scrapy plugin (`scrapy-feedexporter-google-drive`) |
| Google Sheets v4 | `values.append` has no precondition or idempotency key. `batchUpdate` is all-or-nothing. `DeveloperMetadata.metadataId` can be client-chosen and is spreadsheet-unique. | at-least-once. **Inference**: a batch of `[CreateDeveloperMetadata(id = hash(delivery_id)), AppendCells]` could fail whole on a duplicate ID, which would make it idempotent keyed. Needs testing | `google-sheets4` 7.0.0+20251215, 2026-01-01, MIT, community | None | Spider (`google_sheets`), Browse AI, Octoparse, Airbyte (`destination-google-sheets`), Scrapy plugin (`scrapy-feedexporter-google-sheets`) |
| Dropbox | `WriteMode.update(rev)`: overwrite only if rev matches. `add`: never overwrite. Identical content is a no-op without a conflict. `strict_conflict` forces a conflict. | exactly-once fenced (via fence file rev) | `dropbox-sdk` 0.21.0, 2026-10-03, Apache-2.0, official (dropbox org) | None. opendal has a dropbox service with no conditional capability | Octoparse, Scrapy plugin (`scrapy-feedexporter-dropbox`) |
| OneDrive / SharePoint (Graph) | `if-match` / `if-none-match` with eTag or cTag (412). `@microsoft.graph.conflictBehavior=fail` on create; the default for PUT is replace; it goes in the URL. Upload sessions default to `fail`, and a name clash returns 409 `nameAlreadyExists`. | exactly-once fenced | `graph-rs-sdk` 3.0.1, 2025-04-20, MIT, community. Microsoft lists no Rust Graph SDK. `opendal` onedrive declares `write_with_if_match` | None official (Microsoft Dev Proxy can mock Graph; **inference**, not checked) | Scrapy plugin (`scrapy-feedexporter-onedrive`) |
| Box | `If-Match: <etag>` on upload of a new version (412 on mismatch). Upload of a new file returns 409 if it exists. Metadata update uses JSON-Patch with an atomic `test` op. | exactly-once fenced (`test` can compare an epoch in metadata) | No official Rust SDK found. `box-open-sdk` 0.4.4, 2026-08-26, MIT, unofficial. Mostly HTTP API only | None | None found |
| Airtable | `performUpsert` with `fieldsToMergeOn` (1 to 3 fields). 0 matches creates, 1 updates, more than 1 fails. No version precondition. | idempotent keyed (delivery_id field as merge key) | HTTP API only in practice. `airtable-api` 0.1.36 (2022-06-03, Oxide) is stale | None | Apify, Browse AI |
| Notion | No idempotency key, no conditional request, no upsert on page create. A shared 409 schema lists `idempotency_key_reused`, but pages document no way to send a key. | at-least-once | `notion-client` 1.1.1, 2026-04-20, MIT, community. `notionrs` 0.32.0, 2026-08-14, MIT/Apache-2.0, community | None | None found |

## Per-destination notes

### AWS S3
- `If-None-Match: *` fails with 412 if the key exists. Concurrent conditional writes: the first one to finish wins. A concurrent delete can produce 409. https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html
- `If-Match: <ETag>` fails with 412 on mismatch and 404 if the object is gone. It needs SigV4. In-flight multipart uploads are not considered until completion.
- If-None-Match launched in August 2024: https://aws.amazon.com/about-aws/whats-new/2024/08/amazon-s3-conditional-writes/
- If-Match launched in November 2024: https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-functionality-conditional-writes/
- Bucket-policy enforcement uses the `s3:if-match` and `s3:if-none-match` condition keys: https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-enforcement-conditional-write-operations-general-purpose-buckets/
- Conditional copy (Oct 2025) and conditional delete (Sep 2025) followed.
- `aws-sdk-s3` exposes `PutObjectFluentBuilder::if_match` and `if_none_match`. https://docs.rs/aws-sdk-s3/1.152.0/aws_sdk_s3/operation/put_object/builders/struct.PutObjectFluentBuilder.html
- `object_store` `PutMode` has three variants. `Create` returns `Error::AlreadyExists`. `Update(UpdateVersion{e_tag, version})` returns `Error::Precondition`. https://docs.rs/object_store/0.14.2/object_store/enum.PutMode.html
- `S3ConditionalPut` defaults to `ETagMatch` in the v0.14.2 source (`src/aws/precondition.rs`). https://docs.rs/object_store/0.14.2/object_store/aws/enum.S3ConditionalPut.html
- `opendal` s3 declares `write_with_if_match` and `write_with_if_not_exists` (core/services/s3/src/backend.rs, v0.59.4). docs.rs failed to build 0.59.4. 0.58.2 documents the capability flags: https://docs.rs/opendal/0.58.2/opendal/struct.Capability.html
- LocalStack's S3 provider (main) handles `IfNoneMatch` and `IfMatch` on PutObject: https://github.com/localstack/localstack/blob/main/localstack-core/localstack/services/s3/provider.py

### Cloudflare R2
- The S3 API compatibility table marks PutObject with If-Match, If-Modified-Since, If-None-Match and If-Unmodified-Since. CompleteMultipartUpload lists no conditional headers. https://developers.cloudflare.com/r2/api/s3/api/
- R2 extensions add `cf-copy-destination-if-*` for CopyObject (beta, 412). https://developers.cloudflare.com/r2/api/s3/extensions/
- Consequence: use single-part PutObject for fenced writes. **Inference**.

### MinIO
- The MinIO blog says MinIO supports both If-None-Match and If-Match, merged February 2023. https://www.min.io/blog/leading-the-way-minios-conditional-write-feature-for-modern-data-workloads
- The GitHub repo `minio/minio` is archived. Its last release is RELEASE.2025-10-15T17-29-55Z. That matters for long-term CI use. https://github.com/minio/minio
- Alternatives in testcontainers-modules 0.15.0: `rustfs`, `localstack`. RustFS conditional-write support was not verified.

### Google Cloud Storage
- Preconditions: generation, metageneration, 0 = absent, 412 on failure. XML multipart does not support preconditions. https://docs.cloud.google.com/storage/docs/request-preconditions
- The official crate's `WriteObject` has `set_if_generation_match` (0 prevents overwrite) and metageneration variants. https://docs.rs/google-cloud-storage/1.20.0/google_cloud_storage/builder/storage/struct.WriteObject.html
- The fake-gcs-server upload handler applies generation preconditions at finalize: https://github.com/fsouza/fake-gcs-server/blob/main/fakestorage/upload.go

### Azure Blob
- Conditional headers work on Put Blob and Put Block List. Writes fail with 412. https://learn.microsoft.com/en-us/rest/api/storageservices/specifying-conditional-headers-for-blob-service-operations
- `x-ms-if-tags` accepts `=`, `<>`, `>`, `>=`, `<`, `<=`, AND, OR on string tags, compared lexicographically. Zero-pad the epoch.
- Example: `x-ms-if-tags: "epoch" <= '0000000042'` on Put Blob rejects a stale writer. This works only once the blob exists. **Inference** (composition of documented features).
- Upload options in `azure_storage_blob` include `if_tags`: https://docs.rs/azure_storage_blob/1.1.0/azure_storage_blob/models/struct.BlockBlobClientUploadOptions.html

### Backblaze B2, Wasabi, DigitalOcean Spaces
- B2 S3 PutObject lists no conditional headers: https://www.backblaze.com/apidocs/s3-put-object
- B2's compatibility page does not mention them: https://www.backblaze.com/docs/cloud-storage-s3-compatible-api
- Proxmox patch (third party) reports B2 rejects `If-None-Match` with 501: https://lists.proxmox.com/pipermail/pbs-devel/2025-July/014234.html
- No Wasabi documentation on conditional writes was found.
- Spaces: https://docs.digitalocean.com/products/spaces/reference/s3-compatibility/ does not mention conditional headers. Unsupported operations return `NotImplemented`.
- Treat all three as key-only (delivery_id + epoch in the key). Probe at connect time: send `If-None-Match: *` to an existing key and expect 412. **Inference**.

### Tigris
- If-Match, If-None-Match and If-Unmodified-Since on PUT. 412 on failure. Global and Dual-region buckets are only eventually consistent across regions. https://www.tigrisdata.com/docs/objects/conditionals/

### Ceph RGW
- The source reads If-Match and If-None-Match for PutObject and CompleteMultipart: https://github.com/ceph/ceph/blob/main/src/rgw/rgw_rest_s3.cc
- The docs feature table is silent: https://docs.ceph.com/en/latest/radosgw/s3/
- Version coverage is unverified. **Inference**.

### SFTP
- SFTP draft-02 defines `SSH_FXF_EXCL` ("fail if the named file already exists"). Its RENAME says "It is an error if there already exists a file with the name specified by newpath". https://www.ietf.org/archive/id/draft-ietf-secsh-filexfer-02.txt
- OpenSSH extensions are `posix-rename@openssh.com` (POSIX rename, atomic overwrite) and `hardlink@openssh.com` (link(2)). https://github.com/openssh/openssh-portable/blob/master/PROTOCOL
- Recommended publish: write `tmp/<delivery>.<epoch>.part`, fsync (`fsync@openssh.com`), then hardlink or rename without overwrite to the final name, so a duplicate fails cleanly. **Inference**.
- No CAS, so a stale epoch cannot be rejected by the server.
- `openssh-sftp-client` `Fs::rename` uses posix-rename when the server supports it, and it has `hard_link`. Its `OpenOptions::create_new` sends EXCL. https://github.com/openssh-rust/openssh-sftp-client
- `russh-sftp` has `OpenFlags::EXCLUDE` and a `hardlink` extension, but no posix-rename helper on master. https://github.com/AspectUnk/russh-sftp
- `ssh2` `RenameFlags` (OVERWRITE, ATOMIC, NATIVE) map to SFTP v5+ flags. OpenSSH speaks v3, so expect them to be ignored. **Inference**.
- opendal sftp uses openssh-sftp-client `create_new` for `if_not_exists` (core/services/sftp/src/reader.rs, v0.59.4).

### FTP/FTPS
- RFC 959 defines RNFR/RNTO and STOU. STOU picks a server-unique name, so it is not deterministic. https://www.rfc-editor.org/rfc/rfc959.txt
- RFC 959 does not specify RNTO-over-existing behavior; it varies by server. **Inference**.
- suppaftp has no STOU enum. Use `custom_command`. https://github.com/veeso/suppaftp
- Scrapy FTP feeds overwrite by default: https://docs.scrapy.org/en/latest/topics/feed-exports.html

### Google Drive v3
- The discovery doc (revision 20261005) lists `files.update` parameters with no precondition. File has no `etag`. `version` and `headRevisionId` are output-only. https://www.googleapis.com/discovery/v1/apis/drive/v3/rest
- A pre-generated ID gives an idempotent create: "subsequent retries return a 409 Conflict HTTP status code response and duplicate files aren't created". https://developers.google.com/workspace/drive/api/guides/create-file
- The generated ID must be persisted with the delivery, because the client cannot choose an arbitrary ID. https://developers.google.com/workspace/drive/api/reference/rest/v3/files/generateIds

### Google Sheets v4
- The discovery doc (revision 20261005) shows `values.append` parameters with no precondition. `BatchUpdateSpreadsheetRequest` has no write-control field. "If any request is not valid, no requests will be applied." https://www.googleapis.com/discovery/v1/apis/sheets/v4/rest
- The `DeveloperMetadata.metadataId` description says "IDs may be specified when metadata is created".
- The developer-metadata dedup trick above is **inference**.
- The Docs API has `WriteControl.requiredRevisionId`; Sheets does not.

### Dropbox
- `WriteMode` (add, overwrite, update Rev) and `strict_conflict` are defined in the official API spec: https://github.com/dropbox/dropbox-api-spec/blob/main/files.stone
- The Rust SDK is from the dropbox org and was updated 2026-10-03: https://github.com/dropbox/dropbox-sdk-rust

### OneDrive / SharePoint
- `if-match` / `if-none-match` with eTag or cTag return 412 on mismatch. Default `conflictBehavior` is `fail`. A name clash at commit returns 409 `nameAlreadyExists`. https://learn.microsoft.com/en-us/graph/api/driveitem-createuploadsession?view=graph-rest-1.0
- `conflictBehavior` values are fail, replace, rename. The PUT default is replace. eTag covers metadata + content; cTag covers content only. https://learn.microsoft.com/en-us/graph/api/resources/driveitem?view=graph-rest-1.0
- No Rust Graph SDK is listed: https://learn.microsoft.com/en-us/graph/sdks/sdks-overview

### Box
- Upload new version: `if-match` takes the etag, 412 on mismatch. https://developer.box.com/reference/post-files-id-content/
- Upload new file: 409 if the file exists. https://developer.box.com/reference/post-files-content/
- If-Match semantics are described here: https://developer.box.com/guides/api-calls/ensure-consistency/
- Metadata update: JSON-Patch `test` op, applied atomically. https://developer.box.com/reference/put-files-id-metadata-id-id/ The error status for a failed `test` is not documented.

### Airtable
- Upsert semantics come from https://airtable.com/developers/web/api/update-multiple-records. No ETag or revision precondition.

### Notion
- Page create has no idempotency, conditional or upsert parameters. Notion-Version is `2026-03-11`. https://developers.notion.com/reference/post-page
- 409 `conflict_error`: "data collision", retry. https://developers.notion.com/reference/status-codes
- Dedup would need query-then-create, which is not atomic. **Inference**.

## Vendor quick pass (first-party sinks)
- **Scrapy built-in** `FEED_STORAGES_BASE`: file, stdout, s3, gs, ftp, ftps. S3 and GCS cannot append. https://docs.scrapy.org/en/latest/topics/feed-exports.html
- **scrapy-plugins** org exporters: sftp, azure-storage, dropbox, google-drive, google-sheets, onedrive. https://github.com/scrapy-plugins
- **Zyte**: S3, GCS and SFTP guides, all through Scrapy feeds. https://docs.zyte.com/web-scraping/guides/export/index.html
- **Apify** integrations page: Snowflake, Airtable, Google Drive, HubSpot, n8n, Make, Zapier, Gumloop, webhooks. No S3, Sheets, Dropbox, SFTP or Notion is listed. Store actors were not checked. https://docs.apify.com/platform/integrations
- **Bright Data** delivery: API download, webhook, S3, GCS, Snowflake, Pub/Sub, Azure Storage, SFTP/FTP. No email listed. https://docs.brightdata.com/datasets/scrapers/scrapers-library/delivery-options
- **Spider** data connectors: S3 (keyed by domain + timestamp, optional prefix), GCS, Google Sheets (append), Azure Blob, Supabase. https://spider.cloud/docs/core/data-connectors/
- **Firecrawl**: webhooks only (3 retries at 1, 5 and 15 minutes). No storage sinks. https://docs.firecrawl.dev/webhooks/overview
- **Browse AI**: Google Sheets, Airtable, S3, webhooks, Zapier, Make, Pabbly, Integrately, JSON/CSV. https://www.browse.ai/integrations
- **Octoparse**: Google Sheets, MySQL, SQL Server, PostgreSQL, Oracle. Google Drive, S3 and Dropbox from 8.7.4 on the Professional plan and above. File exports; Zapier for others. https://helpcenter.octoparse.com/en/articles/10221225-export-data-to-cloud-storage
- **Airbyte** destination connectors in airbytehq/airbyte: s3, s3-data-lake, gcs, gcs-data-lake, azure-blob-storage, r2, sftp-json, google-sheets. None for Drive, Dropbox, OneDrive, Box, Airtable or Notion. https://github.com/airbytehq/airbyte/tree/master/airbyte-integrations/connectors
- **dlt** filesystem destination: s3 (plus MinIO, R2 and Tigris via endpoint_url), gs, az/abfss, sftp, file, hf. The default layout `{table_name}/{load_id}.{file_id}.{ext}` puts the load ID in the key. https://dlthub.com/docs/dlt-ecosystem/destinations/filesystem
