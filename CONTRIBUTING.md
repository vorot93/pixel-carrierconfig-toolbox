# Contributing to pixel-carrierconfig-toolbox

Working guide for developers and AI agents. For architecture see
[DESIGN.md](DESIGN.md); for end-user behavior see [README.md](README.md).

---

## Toolchain

- **Edition:** 2024 (Rust ≥ 1.85; no pinned MSRV).
- **System SQLite is a required build dependency.** `rusqlite` is built without the
  `bundled` feature, so a system `libsqlite3` must be present at build time.
  - Debian / Ubuntu: `sudo apt install libsqlite3-dev`
  - macOS Homebrew: `brew install sqlite`
- **`Cargo.lock` is gitignored** (not committed). `phf` is left unpinned.

---

## Dependencies

| Crate | Version in `Cargo.toml` | Feature flags | Rationale |
|---|---|---|---|
| `anyhow` | `1` | — | Application-level errors in the binary and `report` module; `bail!` / `.context()` |
| `thiserror` | `2` | — | Library error type (`Error` enum in `src/error.rs`); implements `std::error::Error` |
| `clap` | `4` | `derive` | CLI argument parsing with derive macros (`#[derive(Parser, Subcommand)]`) |
| `crc32fast` | `1` | — | `id = crc32(name)` for NV items; sole id computation |
| `indexmap` | `2` | `serde` | Insertion-order-preserving map for NV items; preserves TOML key order for stable re-encoding |
| `lz4_flex` | `0.13` | `std` (no defaults) | LZ4 block (de)compression for CLZ4; `default-features = false` disables the frame format |
| `phf` | `0.14` | — | Compile-time perfect hash for the static NV table |
| `rusqlite` | `0.40` | — (non-bundled) | SQLite access; **no `bundled` feature** → system SQLite required |
| `sha2` | `0.11` | — | `content_hash = sha256(bytes)[:20]` |
| `serde` | `1` | `derive` | Serialization derive for TOML models |
| `toml` | `1` | `preserve_order` | TOML parsing/generation; `preserve_order` keeps NV item ordering stable for round-trips |
| `zip` | `8` | (no defaults) | Store-only Magisk module zip; `default-features = false` disables compression backends |

**Protobuf is hand-rolled** (varint codec in `src/confseq.rs`). `prost` is not used
because byte-faithful encode control requires knowing the exact wire layout of every
field, including empty `Val` messages and items with duplicate ids.

---

## Build, test, run

### Build

```sh
cargo build --release
```

The binary is at `target/release/pixel-carrierconfig-toolbox`.

### Corpus-free unit tests

~29 tests in `src/` always run via `cargo test` (no corpus needed). Notable:

- `nv_names_are_crc32_consistent` validates all ~60,093 NV table entries on every run
  (data is compiled in).
- `encode_then_decode_roundtrips` covers byte-faithful ConfSeq codec including empty
  `Val`s and multiple `Val` groups.
- `rewrite_replaces_all_duplicate_occurrences` covers the 4× duplicate manifest case.

### Corpus-gated integration tests

~5 tests in `tests/roundtrip.rs` require `CFGDB_CORPUS` to be set to a real cfgdb
directory. Without it they print `"skip: CFGDB_CORPUS not set"` and pass immediately:

```sh
CFGDB_CORPUS=/path/to/carrierconfig cargo test
```

Covered corpus-gated: full decompile (incl. PEM-cert blob handling), byte-faithful
whole-directory round-trip, edit isolation, confseq decode→encode byte-identity, and
manifest rewrite byte-faithfulness.

### Subcommand-level testing

No corpus needed:

```sh
# Built-in codec sanity checks (crc32, CLZ4, content_hash, ConfSeq encode/decode):
pixel-carrierconfig-toolbox self-test

# Integrity validation of a decompiled project:
pixel-carrierconfig-toolbox check <project-dir>
```

---

## CI

`.github/workflows/main.yml` runs on every push to `master` and every PR:

```
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Corpus is absent in CI; integration tests skip cleanly.

---

## Regenerating the NV table

`src/nvtable_data.rs` is a generated file — never edit it by hand. It is a
`phf::Map<u32, &'static str>` with ~60,093 entries mapping crc32 ids to Shannon NV
item names for the g5400 modem. It is marked `linguist-generated=true -diff` in
`.gitattributes` and carries an `// @generated` header.

**Source:** Shannon NV CSV for the g5400 (external; not committed to this repo).

The recipe — CSV parsing with RFC-4180 unquoting, the `id == crc32(name)` check, a
throwaway `phf_codegen` generator pinned to the resolved `phf` version, and verifying
via `cargo test` — is documented in full at
[docs/regenerating-nv-table.md](docs/regenerating-nv-table.md). The
`nv_names_are_crc32_consistent` unit test is the safety net: it catches any misquoting
or CSV-parsing error that produces a name whose crc32 does not match the declared id.

---

## Workflow conventions

- **Spec-driven development.** New work starts as a spec, is planned, and is executed
  with review checkpoints (cadence: brainstorm → spec → plan → execute-with-review).
  Process artifacts — specs, plans, task briefs/reports, review diffs — live in
  `~/.superpowers/pixel-carrierconfig-toolbox/{specs,plans,sdd}/`, never in the repo
  (the repo's `.gitignore` blocks any in-repo superpowers paths).
- **Corpus-gated testing.** Integration tests require a real cfgdb directory from a
  Pixel device (`CFGDB_CORPUS`). CI runs without the corpus; local development should
  confirm the round-trip with a real corpus before merging any format-touching change.
- **Byte-faithfulness is non-negotiable.** The whole-directory round-trip
  (`roundtrip_is_byte_faithful`) must pass. Any change to encoding logic must be
  verified byte-by-byte against a real corpus before merging.

---

## Load-bearing gotchas

These behaviors are easy to get wrong and silently break the round-trip or device
behavior. Each is enforced somewhere in the code; this section exists so a fresh
contributor knows to expect them.

- **`confmap.carrier_id` is TEXT in the schema.** All joins must use
  `CAST(carrier_id AS INTEGER)` or queries return no results. See `cfgdb.rs` SQL.
- **`versions.confpack` is TEXT stored in an INTEGER-declared column.** Read via
  `rusqlite::types::Value` to preserve the text label (e.g. `"cfgdb-…"`) rather than
  coercing to `0`. Versions are stored as `Vec<(String, String)>` pairs, not typed
  integers.
- **RFC-4180 quoted NV names.** The g5400 Shannon NV CSV has 13 rows with embedded
  commas whose names are double-quoted. The generator must strip exactly one layer of
  RFC-4180 quoting to recover the true NV name before computing its crc32 id. This
  matters only when regenerating the table — see
  [docs/regenerating-nv-table.md](docs/regenerating-nv-table.md).
- **Nested `Val` groups, not flat.** NV item values are `Vec<Vec<i64>>` — one inner
  `Vec` per `Val` submessage on the wire. Flattening to `Vec<i64>` loses `Val`
  boundaries and breaks byte-faithful re-encoding. In TOML: `NAME = [[4097]]` (one
  group, one value), `NAME = [[1, 2], [3]]` (two groups), or `NAME = [[]]` (an empty
  `Val` submessage, `12 00` on the wire).
- **Repeated-id and undecodable confseqs become `blob:` locks.** A confseq whose wire
  format has two NV items with the same id, or one that cannot be protobuf-decoded
  (e.g. a PEM certificate), is stored verbatim as `blob:<hash>` and never exposed as
  an editable TOML module. Do not attempt to parse or re-encode these.
- **One confseq may be listed 4× per carrier.** The `confman_<hash>` table preserves
  duplicates (row order matters). The compiler's remap patches all occurrences
  atomically; the manifest rewriter replaces all matching `12 14 <hash>` windows. Do
  not deduplicate.
- **Shared-confman divergent edits are rejected in P1.** If two carriers share a
  confman and the user edits them to produce different confseqs for the same original,
  the compiler returns an error ("needs P2 confman-forking"). This is a known P1
  limitation; see [DESIGN.md](DESIGN.md) for the roadmap.
- **`lz4_flex` block recompression is not byte-stable.** `lz4_flex::block::compress`
  produces different bytes than the original compressor. CLZ4 confseqs are therefore
  always stored verbatim in `originals/`; the decompress path is used only for content
  comparison, not for round-trip output.
