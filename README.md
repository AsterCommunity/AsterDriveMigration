# AsterDriveMigration

Design documents:

- [Cloudreve and AsterDrive field mapping](docs/cloudreve-to-asterdrive-field-mapping.md)
- [Migration architecture](docs/migration-architecture.md)

Database migration tool for moving a Cloudreve v4 installation to AsterDrive (AD).

## Supported data

- Cloudreve groups to AD storage policy groups
- users and profiles, including quotas and disabled status
- local, S3, KS3 and standard-endpoint Tencent COS storage policies
- folders, files, physical entities/blobs and version history
- file/folder metadata, including Cloudreve v4 `tag:*` metadata as AD tags
- public shares and regenerated AD v2 direct-link mappings

Cloudreve password hashes are not compatible with AD. The migration assigns the supplied temporary password to every migrated user and sets `must_change_password = true`. Passkeys, WebDAV credentials, two-factor secrets, OAuth grants, sessions and filesystem events are not migrated.

Cloudreve tasks are runtime state rather than portable business data. The migration reports their source count and intentionally leaves AD `background_tasks` empty so AD can create its own runtime tasks.

By default the tool reuses existing storage objects. For local storage, pass the directory from which Cloudreve entity paths should be resolved. Compatible object-storage policies keep their existing bucket, endpoint, credentials and object keys. This is a zero-copy migration and is the preferred choice when AD can safely access the same storage backend.

## Prerequisites

1. Stop writes to Cloudreve and back up both databases.
2. Create an empty AD database and run the AD database migrations against it.
3. Do not start the AD application against the target database until this migration completes.

## Check compatibility

```powershell
cargo run -- check `
  --source-url "sqlite://C:/cloudreve/cloudreve.db?mode=ro" `
  --target-url "sqlite://C:/asterdrive/asterdrive.db" `
  --report-path "C:/migration/cloudreve-preflight.json"
```

The URLs can also be provided through `CLOUDREVE_DATABASE_URL` and `ASTERDRIVE_DATABASE_URL`. SQLite, MySQL and PostgreSQL URLs supported by SeaORM can be used.

`check` now performs source preflight before reporting compatibility. It checks folder cycles, orphaned file/entity/metadata/share/direct-link relations, missing storage policies, negative sizes/counters and duplicate active-user emails. Failed checks are included in the JSON report; `migrate` runs the same checks again and refuses to write the target database until they pass.

## Run migration

```powershell
$env:ASTER_MIGRATION_DEFAULT_PASSWORD = "replace-with-a-strong-temporary-password"
$env:ASTER_DIRECT_LINK_SECRET = "use-the-same-auth.direct_link_secret-as-ad"

cargo run -- migrate `
  --source-url "sqlite://C:/cloudreve/cloudreve.db?mode=ro" `
  --target-url "sqlite://C:/asterdrive/asterdrive.db" `
  --local-base-path "C:/cloudreve" `
  --run-id "cloudreve-cutover-2026-07-13" `
  --report-path "C:/migration/cloudreve-to-ad.json"
```

`ASTER_DIRECT_LINK_SECRET` is optional only when there are no direct links to regenerate. When supplied, each active Cloudreve `direct_links` row is mapped to a newly signed AD `/d/v2...` URL and stored under the target file's `cloudreve.direct_links` properties. Existing Cloudreve `/f/...` URLs cannot remain valid because the URL path, ID and signing algorithms differ.

Use `--dry-run` to perform preflight checks without writing the target. The target core tables must be empty by default. `--allow-non-empty-target` disables that guard but may still fail on unique values or conflicting data.

Cloudreve native Alibaba OSS, Huawei OBS, Qiniu, Upyun, remote-node and encrypted storage policies cannot be reused safely by AD. Tencent COS policies using a custom domain or a bucket-mismatched endpoint are also unsupported. The migration stops when they are present. `--skip-unsupported-policies` explicitly omits those policies and all dependent files. Independently encrypted entities, identified by `entities.recycle_options.encrypt_metadata`, are always skipped together with files whose current entity is encrypted. Both blob and file source IDs and their fixed skip reasons are included in terminal and JSON migration reports; encryption keys and IVs are never copied into the report.

Compatible Cloudreve OneDrive policies are migrated as AsterDrive OneDrive policies. Global/China Microsoft Graph endpoints, default drives, explicit drive IDs, SharePoint site drives, group drives, chunk size, and relay/direct transfer topology are preserved. Cloudreve refresh tokens, Client Secrets, and OAuth callback state are not copied into AsterDrive's encrypted credential tables. Configure the Microsoft Graph application and authorize each migrated OneDrive policy in the AsterDrive admin console before cutover; after authorization, existing Cloudreve `entities.source` paths are reused without copying object bytes.

## Reuse local storage

For compatible Cloudreve local storage, migration configures AD to read the same files through the target policy base path. Relative `entities.source` values are preserved after rejecting unsafe parent-directory segments. Absolute values must be inside the configured local root and are converted to relative AD `storage_path` values. This avoids copying object bytes and prevents AD from joining a base path with a second absolute-path prefix.

`--local-base-path` is the fallback root for every local policy. When policies use different volumes, pass one or more explicit source-policy mappings:

```powershell
cargo run -- migrate `
  --source-url "sqlite://C:/cloudreve/cloudreve.db?mode=ro" `
  --target-url "sqlite://C:/asterdrive/asterdrive.db" `
  --default-password "change-this-password" `
  --local-base-path "D:/cloudreve-default" `
  --local-policy-root "1=D:/cloudreve-default" `
  --local-policy-root "2=E:/cloudreve-archive" `
  --verify-local-storage
```

`--verify-local-storage` checks each compatible local policy root and every migrated local blob's resolved path, regular-file status, open permission and byte length. With `--dry-run`, it scans all eligible local blobs before any target writes. It does not verify S3-compatible buckets or prove that a running AD instance can access a Docker mount; validate those separately before cutover.

`--verify-remote-storage` enables provider-level verification for compatible Cloudreve `s3`, `ks3` and standard-endpoint `cos` policies before each migrated blob is committed. It performs an authenticated `HeadObject` and validates the reported size, then performs an authenticated `bytes=0-0` range read for non-empty objects. With `--dry-run`, the full eligible remote-object set is verified before target writes begin. This verifies the source credentials, endpoint, bucket, object key, signing region and read path; it does not copy remote bytes or validate a running AD HTTP service. Explicit Cloudreve S3/KS3 regions are migrated into AsterDrive's `options.s3_region`; policies without one use AsterDrive's `auto` default.

## Object storage boundary

The migration writes database metadata and preserves each compatible Cloudreve object key. It does not move local or remote object bytes. When AsterDrive must use a different filesystem root, bucket or endpoint, transfer the objects separately with an infrastructure tool such as `rsync`, `rclone`, a filesystem snapshot or a provider-side copy job, while preserving the relative `storage_path` layout. Complete that transfer and its byte-level verification before cutover.

## Limited resume

Migration stages run in this order: policies, policy groups, users, folders, blobs, files, metadata, shares and direct links. Most stages are committed as one transaction. Blobs use keyset pages ordered by Cloudreve `entities.id`, while files use keyset pages ordered by Cloudreve `files.id`; each page writes target records, object mappings, its cursor and report progress in one target transaction.

Use a stable run ID for an operational migration. If a stage fails, fix the external cause and rerun the same command with `--resume`:

```powershell
cargo run -- migrate `
  --source-url "sqlite://C:/cloudreve/cloudreve.db?mode=ro" `
  --target-url "sqlite://C:/asterdrive/asterdrive.db" `
  --local-base-path "C:/cloudreve" `
  --run-id "cloudreve-cutover-2026-07-13" `
  --blob-batch-size 500 `
  --file-batch-size 500 `
  --resume `
  --report-path "C:/migration/cloudreve-to-ad.json"
```

Resume verifies the source URL/count fingerprint, target URL fingerprint and migration-plan fingerprint. Repeat the same password, direct-link secret and migration flags used by the original run. Completed stages are skipped and their source-to-target mappings are restored from the checkpoint. Blob batch size is operational only and may be adjusted when resuming.

`--blob-batch-size` and `--file-batch-size` default to 500 and accept 1-10000. A failed blob or file page rolls back only that page; resume continues after the last committed entity or file ID. Blob and normal-file source rows are no longer loaded into memory as complete collections, and their mappings are stored row-by-row in `aster_external_migration_object_map`.

During migration the CLI writes progress to stderr: each stage emits start/completed markers, while committed blob/file pages include processed source rows, the exact stage total, batch row count and batch bytes. Progress is printed only after the corresponding target transaction commits, so it never claims an uncommitted page as complete.

Folders, metadata, shares and direct links are still stage-level only: a failure restarts that entire stage. A file page loads only its related entity/version rows into memory. Object-byte transfer remains outside the migration process.

## JSON migration report

`--report-path` writes a pretty-printed JSON report for both `check` and `migrate`. A completed migration report contains:

- source and migrated counts for every supported object type
- skipped counts grouped by type, plus source ID and reason for every skipped object
- sorted source-to-target ID mappings for policies, groups, users, folders, blobs, files and shares
- every Cloudreve tag metadata assignment and its AD tag/entity IDs
- every regenerated direct-link URL and its Cloudreve/AD file IDs
- post-commit database count checks, tag-binding checks and direct-link property checks
- final relation checks for folders, blobs, files, versions and shares; automatic `ref_count` and `storage_used` recalculation using AD's current-file-plus-history accounting rule
- when `--verify-local-storage` is used, local AD object open/read and size checks
- run ID, whether the execution resumed, and the list of completed stages

The CLI writes the JSON report before returning a validation error. A failed post-migration check therefore produces a usable report and exits with a non-zero status. Direct-link URLs are bearer-style public capabilities, so the report file must be stored with restricted access.

## Migration operations

Use these target-database commands to inspect an existing checkpoint without rerunning the migration:

```powershell
# List run ID, status, latest completed stage and update time.
cargo run -- list --target-url "<AD_DATABASE_URL>"

# Print the stored report for a run.
cargo run -- status --target-url "<AD_DATABASE_URL>" --run-id "cloudreve-cutover-2026-07-23"

# Export a fresh JSON report and a capability-free source-to-target ID CSV.
cargo run -- report `
  --target-url "<AD_DATABASE_URL>" `
  --run-id "cloudreve-cutover-2026-07-23" `
  --report-path "C:/migration/report.json" `
  --csv-mapping-path "C:/migration/mappings.csv"
```

`resume` is an explicit alias for `migrate --resume`; it still requires the original source URL, target URL, temporary password, run ID and every plan-affecting option to match the stored checkpoint. `abort` only marks a `running` or `failed` checkpoint as aborted and never rolls back migrated AD business records. `cleanup --confirm` deletes **only a completed run's migration checkpoint, cursors and object mappings**; it never deletes AD files, blobs, users or source data, and makes that run impossible to resume.

Committed blob/file progress lines now include elapsed seconds, committed rows per second and a row-based ETA. These estimates are intentionally emitted only after the matching database transaction commits.

## Verification before push

Every migration feature must include focused tests plus end-to-end coverage where it writes database state. Before pushing, run and require all of these commands to pass:

```powershell
cargo fmt -- --check
cargo check --offline
cargo clippy --offline --all-targets --all-features -- -D warnings
cargo test --offline --all-targets --all-features
```
