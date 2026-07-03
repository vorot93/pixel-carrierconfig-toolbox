# Design — pixel-carrierconfig-toolbox

Architecture and invariants of the cfgdb decompile/recompile tool. For working conventions see [CONTRIBUTING.md](CONTRIBUTING.md); for end-user behavior see [README.md](README.md).

## Orientation

### Crate layout

`src/main.rs` is a thin binary that delegates everything to the library crate (`src/lib.rs`). All modules live under the library. The integration test crate (`tests/roundtrip.rs`) imports the library as `pixel_carrierconfig_toolbox`.

### Mental model

The tool manages three coupled stores:

1. A **content-addressed confseq store** (`confseqs/` directory, files named by their `sha256(bytes)[:20]` hash, 40 lowercase hex). Confseqs are protobuf-encoded modem NV-item sets.
2. A **SQLite index** (`cfg.db`) containing carrier metadata and `confman_<hash>` tables that map carriers to their ordered list of confseq hashes (duplicates preserved).
3. An **editable TOML projection** (the "project"): one `.toml` file per carrier under `carriers/`, plus `_meta.toml` and `originals/`.

A `(carrier, module) → orig_hash` **lock table** (`_meta.toml` → `locks`) routes each editable module back to its original confseq hash, enabling selective re-encoding at compile time: only confseqs whose decoded content changed are re-encoded; unchanged ones ride through verbatim.

## Repository map

| Module | Responsibility |
|---|---|
| `error` | Crate error type (`thiserror` lib error: `Io`, `Sqlite`, `Confseq`, `Clz4`, `Project`) |
| `project::mod` | TOML data model (`CarrierFile`, `ModuleToml`, `Meta`, `Lock`); `item_key` and `key_to_id` helpers |
| `project::decompile` | cfgdb → project: decode confseqs, classify flavors, write carrier TOMLs, record locks |
| `project::compile` | project → cfgdb: re-encode changed confseqs, update `confman_<hash>` tables, rewrite manifest refs |
| `report` | `inspect` (show carrier NV items), `check` (integrity validation), `self_test` (codec sanity) |
| `magisk` | Build a store-only Magisk module zip in memory (~2.37 MB) |
| `nvtable` | CRC32 ↔ NV-item-name accessor; `crc32_id(name)` is the sole id computation |
| `nvtable_data` | **Generated — never hand-edit.** Static `phf::Map<u32, &'static str>` of ~60,093 entries, label `g5400_nv` |
| `cfgdb` | Read-only `cfg.db` access: load carriers, matching rules, versions, query `confman_<hash>` tables |
| `confseq` | Hand-rolled varint protobuf codec (`ConfSeq`, `NvItem`); `content_hash = sha256(bytes)[:20]` |
| `clz4` | CLZ4 container: 16-byte header (`CLZ4` magic + sizes + zeroed checksum), LZ4 block |
| `manifest` | Decode manifest protobuf (refs only); surgical `12 14 <20 bytes>` ref-hash rewrite |

## Architecture & data flow

### Decompile (cfgdb → project)

1. Open `cfg.db` read-only; load carriers, their matching rules, and the `versions` table.
2. For each carrier, query its `confman_<hash>` table to get the ordered list of confseq hashes. For each hash, read `confseqs/<hash>` and classify it into one of five flavors:

   | Flavor | Condition | TOML output | Stored in `originals/`? |
   |---|---|---|---|
   | Plain re-encodable | Decode succeeds; no duplicate NV ids; not CLZ4 | Yes (editable) | No — re-encodes byte-faithfully from TOML |
   | CLZ4 re-encodable | Decode succeeds; no duplicate NV ids; header = `CLZ4` | Yes (editable) | **Yes** — lz4 recompression is not byte-stable |
   | Repeated NV id | Decode succeeds but name-keyed map loses a duplicate | No — `blob:<hash>` lock | Yes |
   | Undecodable blob | Decode fails (e.g. PEM cert) | No — `blob:<hash>` lock | Yes |
   | Orphan | Not referenced by any carrier | No | Yes |

3. Write `carriers/<slug>.toml` for each carrier (NV item values as `Vec<Vec<i64>>`).
4. Write `_meta.toml` with `locks` (one `{carrier, module, orig_hash}` per editable module or blob lock).
5. Copy `cfg.db`, `manifests/`, and auxiliary files into `source/` for self-contained recompilation.

### Compile (project → cfgdb)

1. Load `_meta.toml` (locks) and all `carriers/*.toml`.
2. Copy verbatim: `originals/` → `out/confseqs/`; `source/manifests/` → `out/manifests/`; `source/cfg.db` → `out/cfg.db`; auxiliary files; `cfg.sha2` (always copied verbatim — the device algorithm is unverified and recompute is unsupported).
3. For each editable module, determine whether it changed:
   - **Stored** (`originals/<orig_hash>` exists — CLZ4, blob, orphan): decode original; compare decoded content with rebuilt TOML. If changed: re-encode (with CLZ4 wrap if `compressed = true`), write new file, record remap `(confman, orig_hash) → new_hash`.
   - **Not stored** (plain — `originals/<orig_hash>` absent): re-encode from TOML (byte-faithful for plain). If `new_hash ≠ orig_hash`: record remap.
4. Apply the remap to each affected `confman_<hash>`:
   - **Manifest**: surgically swap `12 14 <old 20 bytes>` → `12 14 <new 20 bytes>` in the original manifest bytes, preserving all unmodeled fields and keeping the same filename.
   - **`cfg.db`**: `UPDATE confman_{confman} SET confseq = new WHERE confseq = old`.
5. Both interpolation sites validate `confman` as exactly 40 lowercase hex chars before any use (guards against SQL injection via table-name interpolation and path traversal from a crafted `cfg.db`).

## On-device format (overview)

A cfgdb directory is the on-disk unit the tool round-trips. The canonical format reference is [docs/pixel-carrier-modem-config.md](docs/pixel-carrier-modem-config.md); the protobuf wire schema is [docs/cfgdb.proto](docs/cfgdb.proto). At a glance, it consists of:

- **`cfg.db`** — SQLite database: carrier metadata (SIM matching rules, slug/display names, version rows) and one `confman_<40hex>` table per confman listing that confman's ordered confseq hashes (duplicates preserved).
- **`confseqs/<40hex>`** — protobuf-encoded NV-item sets (plain or CLZ4-wrapped), named by `sha256(bytes)[:20]`. NV item `id = crc32(name)`.
- **`manifests/<confman-hash>`** — one protobuf per confman mirroring its confseq list; field-5 ref entries embed each 20-byte confseq hash at `12 14 <bytes>`.
- **`cfg.sha2`** + aux files (`build.info`, `release-label`, `confseqs_symbolic_link_mapping`, `manifests_symbolic_link_mapping`) — copied verbatim; the device-side `cfg.sha2` algorithm is unverified.

The CLZ4 container is a 16-byte header — bytes 0–3 `CLZ4` magic, bytes 4–7 uncompressed size (little-endian u32), bytes 8–11 compressed size (little-endian u32), bytes 12–15 checksum (zeroed; not verified by firmware) — followed by a raw LZ4 block. See the canonical reference for field-level detail.

## Invariants

These must hold at all times. Break any of them and the round-trip or device behavior will fail.

- **Byte-faithful whole-directory round-trip.** A decompile → compile with no edits must reproduce every file in `confseqs/`, `manifests/`, and `cfg.db` byte-for-byte. Verified by `roundtrip_is_byte_faithful` (corpus-gated).
- **`id == crc32(name)` for every NV entry.** Unit-tested over all ~60,093 rows in the bundled table (`nv_names_are_crc32_consistent`). Never create a mapping that violates this.
- **`content_hash = sha256(bytes)[:20]` (40 lowercase hex).** The confseq filename, the key in `confman_<hash>` tables, and the ref hash in manifests — all derived from this formula. Confirmed by `content_hash_is_40_hex` unit test.
- **Edit isolation.** An edit to one carrier's module must not change any other carrier's confseq hashes. Enforced by the confman uniqueness check; deferred for shared confmans (P2).
- **`confman_<hash>` table ≡ `manifests/<hash>` refs as a multiset.** If a confseq appears 4× in the confman table it must appear 4× in the manifest. The `check` subcommand validates this invariant.
- **Manifests are never re-encoded.** Only surgical `12 14 <20 bytes>` hash swaps are applied; all other bytes (unmodeled ref flags, optional `carrier_id`, ordering) are preserved untouched.
- **Plain confseqs re-encode byte-faithfully.** The `Vec<Vec<i64>>` model preserves `Val` grouping (including empty `Val`s encoded as `12 00`), making decode → encode bit-for-bit identical. Verified by `all_confseqs_reencode_byte_identical` (corpus-gated). Because of this guarantee, plain confseqs are NOT stored in `originals/`.
- **CLZ4 confseqs are always stored in `originals/`.** `lz4_flex` block compression is not byte-stable across runs; the original compressed bytes must be kept verbatim.
- **`confman` must be exactly 40 lowercase hex chars** at both interpolation sites (SQL table-name interpolation in `cfg.db` and filesystem path construction for manifests). Both sites validate this before any use.
- **Preserve NV-item order in TOML.** `ModuleToml.items` is an `IndexMap` (insertion-order preserving) to enable byte-faithful re-encoding. Do not convert to `BTreeMap` or `HashMap`.
- **`src/nvtable_data.rs` is generated — never hand-edit.** Marked `linguist-generated=true` in `.gitattributes`. Regenerate per the recipe in [docs/regenerating-nv-table.md](docs/regenerating-nv-table.md).

## Scope & roadmap

**P1 (current — implemented):** Edit existing NV-item values only. Carriers whose confseqs use a confman shared with other carriers ride through unchanged (the compiler re-encodes only what changed, and rejects divergent edits on shared confmans). Carrier matching rules (`carrier_info`) and confman membership are read-only.

**P2 (deferred):**

- Edit carrier matching rules (`carrier_info`).
- Confman membership editing (add/remove confseq modules from a carrier).
- Confman-forking: when two carriers sharing a confman need divergent edits, fork the confman into two independent ones and update both `cfg.db` and manifests accordingly.

**P3 (deferred):**

- New carrier authoring: add a carrier that does not exist in the original `cfg.db`.

**Open unknowns:**

- The device-side algorithm for `cfg.sha2` is unverified. The file is 56 hex chars but its computation is unknown; recomputation is not supported.
- Whether the confman/manifest hash is content-derived (regeneratable from the confseq list), or is an opaque identifier assigned by the build toolchain.
