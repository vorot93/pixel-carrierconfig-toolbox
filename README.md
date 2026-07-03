# pixel-carrierconfig-toolbox

Decompile and recompile the Google Pixel **carrierconfig** (cfgdb) directory —
`cfg.db`, `confseqs/`, `manifests/` — into editable per-carrier TOML files and
recompile them back into a drop-in replacement cfgdb directory or a flashable
Magisk module. The config it edits is the **Shannon modem NV-item** database
(`cfg.db`) — distinct from the Android framework CarrierConfig XML that lives in
`/data/carrier_config/`; these are low-level modem parameters decoded directly by
the Shannon baseband.

> Not affiliated with or endorsed by Google. The file format is observed, not
> officially documented; this tool is for research and personal use.

## ⚠️ Safety & prerequisites

- This tool edits **low-level Shannon modem NV items** flashed to your device via a
  Magisk module. Incorrect values can cause **lost mobile connectivity, failed calls,
  or radio issues**.
- **`cfg.sha2` cannot be re-signed.** The device may reject an edited database;
  `cfg.sha2` is always copied verbatim from the original and cannot be recomputed.
- Requires **root access**, **Magisk**, and a **compatible Pixel device**.
- **Back up the original cfgdb directory before editing.** Keep the original files safe.
- Recovery: disable the Magisk module (via the Magisk app or safe mode) and reboot
  to restore the original config.
- Use only on **your own device**, at your own risk.

## Install

Requires a stable Rust toolchain, edition 2024 (effective floor: Rust ≥ 1.85; no
pinned MSRV).

**System SQLite is a required build dependency.** `rusqlite` is built without the
`bundled` feature, so a system `libsqlite3` must be present:

- Debian / Ubuntu: `sudo apt install libsqlite3-dev`
- macOS Homebrew: `brew install sqlite`

```sh
cargo build --release
# binary at target/release/pixel-carrierconfig-toolbox
```

## Walkthrough

### Step 1 — Obtain a cfgdb directory

On a rooted device, pull the live cfgdb from:

```sh
adb pull /vendor/firmware/carrierconfig
```

Back it up before making any changes.

### Step 2 — Decompile into a project

```sh
pixel-carrierconfig-toolbox decompile /path/to/cfgdb -o project/
```

This produces:
- `project/carriers/<slug>.toml` — one file per carrier; **these are the files you edit**.
- `project/source/` — the original `cfg.db`, manifests, and aux files needed by the
  compiler. Leave this alone.
- `project/originals/` — verbatim copies of confseqs that cannot be rebuilt from a
  TOML (CLZ4-compressed blobs, PEM-cert blobs, and orphans). Leave this alone.
- `project/_meta.toml` — project metadata used by the compiler. Leave this alone.

### Step 3 — Edit a carrier

Open any TOML file in your editor:

```sh
$EDITOR project/carriers/us_vzw.toml
```

**Value format:** Carrier files contain module sections like `[core.sim1]`. Each
NV item maps its name to a list of `Val` groups (nested vectors):

- `TCS_GV_OPT_CARRIER_TYPE = [[4097]]` — one group with one value.
- `[[1, 2], [3]]` — two groups.
- `[[]]` — one empty group.

Strings are stored as integer code-point arrays. Unknown NV ids appear as
`unknown_<crc32id>` and remain fully editable by raw id. Leave the `revision` and
`compressed` fields as-is, and **preserve item order**.

To view a carrier's items without editing, use `inspect`:

```sh
pixel-carrierconfig-toolbox inspect project/ us_vzw
pixel-carrierconfig-toolbox inspect project/ us_vzw --full
```

### Step 4 — Recompile

```sh
pixel-carrierconfig-toolbox compile project/ -o out_cfgdb/
```

`cfg.sha2` is copied verbatim from the original; recomputing it is not supported.
The device may or may not enforce the hash.

Run a validation check before flashing:

```sh
pixel-carrierconfig-toolbox check project/
```

### Step 5 — Package and flash a Magisk module

Compile and produce a Magisk module in one step:

```sh
pixel-carrierconfig-toolbox compile project/ -o out_cfgdb/ --magisk mod.zip
```

Or build the module from an already-compiled cfgdb directory:

```sh
pixel-carrierconfig-toolbox magisk out_cfgdb/ -o mod.zip \
    --dest /vendor/firmware/carrierconfig \
    --name "My carrier NV edits"
```

The resulting module is approximately **2.37 MB** (store-only zip).

Flash `mod.zip` via the **Magisk app** (Modules → Install from storage) and reboot.
Recovery: disable the module in the Magisk app (or via safe mode) and reboot.

## Troubleshooting

**Divergent-edit error ("needs P2 confman-forking"):** Many carriers share a
confman (confseq collection). The compiler rejects a divergent edit — changes that
would apply different values to the same shared confman. Edit those carriers
consistently, or process one at a time.

**`unknown_<id>` items:** Expected for NV ids outside the built-in table. They are
still fully editable by raw id — treat them like any other item.

**No connectivity after flash:** Disable the Magisk module (Magisk app → Modules →
disable) and reboot to restore the original config.

**Build fails — SQLite not found:** Install system SQLite (see [Install](#install)).

**Module is ~2.37 MB:** Expected. The module uses store-only compression.

**Device rejects the edited database:** `cfg.sha2` is always copied verbatim and
cannot be recomputed. The device may or may not enforce the hash; outcome varies by
device and firmware.

## Command reference

| Subcommand | Arguments | Notable flags | Notes |
|---|---|---|---|
| `decompile` | `<cfgdb> -o <dir>` | | |
| `compile` | `<project> -o <dir>` | `--keep-sha2`, `--magisk <zip>`, `--dest <path>` | `--dest` default `/vendor/firmware/carrierconfig`; `--dest` only effective when `--magisk` is also passed; omitting `--keep-sha2` prints a warning to stderr when `cfg.sha2` is present; it is copied verbatim either way |
| `magisk` | `<cfgdb> -o <zip>` | `--dest <path>`, `--name <str>` | `--dest` default `/vendor/firmware/carrierconfig`; `--name` only on `magisk` |
| `inspect` | `<project> <slug>` | `--full` | |
| `check` | `<project>` | | |
| `self-test` | | | |

## Internals

- [CONTRIBUTING.md](CONTRIBUTING.md) — build/test procedures, workflow, gotchas.
- [DESIGN.md](DESIGN.md) — architecture, data flow, invariants.
- [docs/pixel-carrier-modem-config.md](docs/pixel-carrier-modem-config.md) — format reference (cfgdb, confseq encoding, confman tables, manifest invariants).

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0); see [NOTICE](NOTICE).

Not affiliated with or endorsed by Google; the file format is observed, not
officially documented. Editing device configuration is at your own risk.
