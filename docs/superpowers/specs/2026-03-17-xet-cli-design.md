# xet CLI Tool — Design Spec

**Date:** 2026-03-17
**Branch:** `hoytak/260317-xet-cli`
**Crate location:** `xet_pkg/src/bin/xet.rs`

---

## Purpose

A developer/debugging CLI for testing against the Xet CAS server directly — without git or Python.  The primary use cases are:

1. Upload files and inspect deduplication and compression stats.
2. Download files by xet hash and verify correctness.
3. Test against a local on-disk CAS store (using `LocalClient`).
4. Optionally query reconstruction metadata for a given hash.

This is not a user-facing product tool; it is an engineering tool for development and debugging.

---

## Crate

A new binary target added to the existing `xet_pkg` crate:

```
xet_pkg/src/bin/xet.rs
```

`xet_pkg/Cargo.toml` gains a `[[bin]]` entry for `xet`.  No new crate is created.

**Dependency changes required in `xet_pkg/Cargo.toml`:** `clap` (with `derive` feature) and `serde_json` are currently listed under `[dev-dependencies]` only.  They must be moved (or duplicated) into `[dependencies]` so the binary compiles in all profiles.  `tokio`, `xet_data`, and `xet_runtime` are already in `[dependencies]`.

---

## Global Flags

```
xet [OPTIONS] <COMMAND>

OPTIONS:
  --endpoint <URL|PATH>     CAS server endpoint.  Accepts:
                              - https://...         remote server (HF or custom)
                              - /absolute/path      local disk store (normalized to local://)
                              - local:///path       local disk store (explicit form)
                            Defaults to HF_ENDPOINT env var, then the built-in HF default.

  --token <TOKEN>           Auth token for remote endpoints.
                            Defaults to HF_TOKEN env var.  Optional — omit for local endpoints
                            or unauthenticated remote servers.

  -c, --config <KEY=VALUE>  Override a xet_config value.  KEY is <group>.<name>,
                            e.g. client.enable_multirange_fetching=false.
                            May be repeated.
```

---

## Endpoint Resolution

The endpoint string is normalized and then handed to `XetSessionBuilder`, which already understands the `local://` scheme internally (it is used in the existing test suite):

1. If the string starts with `local://` — pass as-is to `XetSessionBuilder::with_endpoint`.
2. If the string is an absolute filesystem path (starts with `/` on Unix) — prepend `local://` and pass to `XetSessionBuilder::with_endpoint`.
3. Otherwise — treat as an HTTPS URL and pass to `XetSessionBuilder::with_endpoint`.  Token is read from `--token` / `HF_TOKEN`; a missing token produces a warning but is not a hard error.
4. If `--endpoint` is absent — do not call `with_endpoint`; the builder uses the `HF_ENDPOINT` env var or the built-in HF default.

There is no separate code path for local vs. remote: `XetSessionBuilder` encapsulates both.

---

## Config Overrides

Each `-c key=value` argument is split on the first `=`.  The overrides are applied by chaining `XetConfig::with_config(path, value)` calls (this method takes ownership and returns `Result<Self, ConfigError>`) before the session is built:

```rust
let mut config = XetConfig::new();
for (k, v) in overrides {
    config = config.with_config(&k, &v)?;
}
```

Errors (unknown key, type mismatch) are reported immediately and abort startup.

---

## Commands

### `upload <FILE...>`

Upload one or more files to the CAS endpoint.

```
xet upload [OPTIONS] <FILE>...

OPTIONS:
  --sha256 / --no-sha256    Compute SHA256 during upload.  Default: compute.
                            Maps to Sha256Policy::Compute / Sha256Policy::Skip
                            from xet_data::processing.
  --output <FILE>           Write results as JSON to FILE instead of stdout.
```

**Behaviour:**
- Creates an `UploadCommit` via the session, calls `upload_from_path(path, sha256_policy)` for each file, then calls `commit()`.
- Prints one line per file from the returned `FileMetadata`:
  ```
  file.bin  hash=<hex>  size=<bytes>  sha256=<hex>
  ```
  Note: per-file dedup/compression ratios are not available in `FileMetadata`; session-level aggregate bytes-new vs. bytes-total are printed as a summary line after all files.
- With `--output`, writes a JSON array of `FileMetadata` objects instead.

**Error handling:** A failed individual file is reported as an error line; remaining files continue.  Exit code is non-zero if any file failed.

---

### `download <HASH> <SIZE> <OUTPUT>`

Download the file identified by `<HASH>` and write it to `<OUTPUT>`.

```
xet download <HASH> <SIZE> <OUTPUT>
```

- `HASH` is the hex-encoded MerkleHash of the file.
- `SIZE` is the file size in bytes.  This is required because `DownloadGroup::download_file_to_path` takes a `XetFileInfo` (hash + file_size) and there is no single-field reconstruction query that returns file size directly without summing `unpacked_length` across all reconstruction terms.  The size is printed by `upload` and by `query`, so obtaining it is straightforward in a typical workflow.
- `OUTPUT` is the destination path.  Parent directories are created if needed.
- Prints a single confirmation line on success: `Downloaded <HASH> → <OUTPUT> (<bytes> bytes)`.
- Exit code is non-zero on failure.

---

### `stats <FILE...>`

Dry-run deduplication and compression analysis.  No data is uploaded.

```
xet stats [OPTIONS] <FILE>...

OPTIONS:
  -r, --recursive           Process directories recursively.
  --output <FILE>           Write results as JSON to FILE.
```

**Behaviour:**
- Runs each file through the chunking and deduplication pipeline without uploading.
- Uses `FileUploadSession` in dry-run mode (analogous to `xtool dedup` with `dry_run=true`, but via `FileUploadSession` rather than `migrate_files_impl` which requires a `HubClient`).
- Prints per-file and aggregate stats:
  ```
  file.bin  chunks=<n>  unique_chunks=<n>  dedup_ratio=<pct>  compressed_size=<bytes>  compression_ratio=<pct>
  ```
- With `--output`, writes a JSON array of per-file stat objects.

---

### `query <HASH> [RANGE]`

Show reconstruction metadata for a file hash.

```
xet query <HASH> [RANGE]
```

- `HASH` is the hex-encoded MerkleHash.
- `RANGE` is an optional `start-end` byte range (inclusive), e.g. `0-1048575`.
- Prints the reconstruction terms: xorb hash, byte offsets, chunk count, and total file size.
- Mirrors the existing `xtool query` command but uses the new endpoint/config infrastructure.

---

## Output Format

| Command    | Default output                | `--output` / JSON available |
|------------|-------------------------------|-----------------------------|
| `upload`   | One line per file + aggregate | Yes                         |
| `download` | One confirmation line         | No                          |
| `stats`    | One line per file + aggregate | Yes                         |
| `query`    | Reconstruction terms          | No (pretty-printed)         |

Progress output (bytes transferred) goes to stderr so it does not interfere with `--output` JSON on stdout.

---

## Implementation Notes

- Use `clap` with derive macros, consistent with the rest of the workspace.
- All session construction goes through `XetSessionBuilder` (from `xet_pkg`); do not call `LocalClient` directly.
- `XetConfig::with_config` takes ownership (`mut self -> Result<Self, _>`); chain calls with `config = config.with_config(k, v)?`.
- `Sha256Policy` is in `xet_data::processing` (re-exported from `xet_pkg` as well).
- For `stats`, use `FileUploadSession` with a flag or configuration that skips the actual CAS upload step.  The exact dry-run entry point should be confirmed during implementation against what `xtool dedup` calls internally.
- For `download`, construct `XetFileInfo` from the `<HASH>` and `<SIZE>` positional arguments.

---

## Out of Scope

- Spinning up a `LocalTestServer` (HTTP server) — use `local://` path for local testing instead.
- Network simulation / congestion control.
- Repository (hub) interaction, commit management, git operations.
- Authentication token refresh.
