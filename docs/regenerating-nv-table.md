# Regenerating the NV table

This is the one-off, out-of-band procedure for regenerating `src/nvtable_data.rs`
— the static `phf::Map<u32, &'static str>` of ~60,093 Shannon NV-item crc32 ids
to names for the g5400 modem. The generated file is committed; the generator
binary and the source CSV are not. Run this only when a new modem firmware drop
ships an updated NV table.

## Source

The sole input is the **Shannon NV CSV for the g5400 modem** — an external file
that is not committed to this repo. It is a two-column `crc32,item` table with a
header row, one entry per NV item. Treat no other source as authoritative for the
table's contents.

## Parsing — RFC-4180 unquoting

For each row after the header, take the name as the last CSV field. The g5400
table has **13 rows whose NV names contain embedded commas**; these names are
RFC-4180-double-quoted in the CSV. Strip **exactly one layer** of surrounding
double-quotes from each field value to recover the true NV name, and do **not**
interpret escape sequences inside the quotes — remove only the surrounding quote
pair. Without this one-layer strip the quote characters stay in the name and
`crc32(name)` no longer matches the declared id.

## The `id == crc32(name)` invariant

Every row declares a crc32 `id` alongside the name. For each `(id, name)` row,
recompute `crc32fast::hash(name.as_bytes())` and verify it equals the declared
`id`. **Discard any row that does not satisfy `id == crc32(name)`** — a mismatch
means the name was mis-parsed (typically a quoting error).

The committed unit test `nv_names_are_crc32_consistent` (in `src/nvtable.rs`) is
the safety net: it iterates every entry in the generated map and re-asserts
`crc32fast::hash(name.as_bytes()) == id`, catching any misquoting or CSV-parsing
error that slipped through generation.

While building the map, dedup by id (exact-duplicate rows collapse into one
entry) and **assert there is no real collision** — two different names hashing to
the same id. The g5400 table is expected to have none; if generation panics on a
collision, that is a genuine ambiguity to resolve by hand, not a silent
last-wins.

## Generator binary

Generate the file with a **throwaway generator binary** — never a committed
crate, never a `build.rs`. In a scratch directory, temporarily add `phf_codegen`
to a throwaway project; the binary reads the CSV, builds the `phf::Map` literal,
and writes `src/nvtable_data.rs` directly.

Pin `phf_codegen` to **the exact resolved version of `phf` in this repo's current
`Cargo.lock`** so the generated `phf::Map` struct literal compiles against the
`phf` crate version the build resolves. Confirm the pinned version with:

```sh
cargo tree -p phf
```

The generator is out-of-band scaffolding: commit only its output
(`src/nvtable_data.rs`), never the generator itself.

## Replacing src/nvtable_data.rs

Overwrite `src/nvtable_data.rs` with the generator's output. The file has a fixed
shape that must be preserved exactly:

- **Line 1** — the `// @generated` header, including the full parenthetical:
  `// @generated (one-off, out-of-band) from the g5400 Shannon NV CSV — DO NOT EDIT.`
- The `#![allow(clippy::all)]` and `#[rustfmt::skip]` attributes, so the large
  literal is neither linted nor reformatted.
- The static must be named `NV_NAMES` with type `phf::Map<u32, &'static str>`.
- The trailing constant `pub static NV_LABEL: &str = "g5400_nv";` at the end of
  the file.

## Verification

After writing the file, run:

```sh
cargo test
```

The test `nv_names_are_crc32_consistent` (in `src/nvtable.rs`) iterates every
entry of `NV_NAMES` and confirms `crc32fast::hash(name.as_bytes()) == id`.
Expected outcome: **60,093 entries validated** and the full crate test suite
passes. If the entry count differs from 60,093, the input CSV or the parsing
step has changed — investigate before committing.

## Marking the file generated

`src/nvtable_data.rs` is marked as generated in `.gitattributes`:

```
src/nvtable_data.rs linguist-generated=true -diff
```

This hides it from GitHub's language stats and excludes it from diffs. Do not
change this marking — the file is machine-generated and should not be reviewed or
edited line-by-line.
