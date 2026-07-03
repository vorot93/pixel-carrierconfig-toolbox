# Pixel CarrierConfig + UECapConfig: System & Modification Playbooks

*'cfgdb' here is Google's per-carrier **modem NV-item** configuration database — distinct from the Android-framework CarrierConfig PersistableBundle (Layer 1); see §1.1.*

## Purpose

This document is the primary deliverable of a corpus-driven, static-analysis investigation into how Pixel devices store, load, and apply per-carrier configuration (the `cfgdb` / CarrierConfig system) and per-carrier UE capability profiles (the `uecapconfig` system). It is intended to give a reader a complete, evidence-backed understanding of both systems and practical playbooks for modifying them — specifically enabling IMS for an unsupported carrier (Playbook A) and authoring band / CA-combo capability profiles (Playbook B).

## Status Legend

Claims in this document carry one of three markers:

- **[established]** — directly verified from the corpus, the toolbox, or a cited public standard (3GPP TS 36.331/38.331, AOSP `CarrierConfigManager`). This is the default; unmarked claims are established.
- **[inferred]** — the evidence strongly supports the claim but direct confirmation is absent; reasoning is given.
- **[unverified]** — the claim is plausible but has not been confirmed by any available evidence; flagged for future work.

## Evidence Policy

Every quantitative claim is derived from the whole corpus, never a sample. Evidence is grounded in (a) the data (`../carrierconfig/`, `../uecapconfig/`), (b) the toolbox / `uecaps_info.py`, or (c) public 3GPP specifications (TS 36.331, TS 38.331) and AOSP `CarrierConfigManager`. Claims not meeting this bar are labelled `[inferred]` or `[unverified]`. Modem and vendor images are opportunistic corroboration only — no firmware RE is performed. Reproducible analysis scripts live under `analysis/` and back every quantitative statement in the text.

---

## Part 1 — System overview

### 1.1 Three-layer model

Pixel phones use three parallel per-carrier configuration layers, two of which are shipped as on-device data stores:

| Layer | Dataset | Consumer | Level |
|-------|---------|----------|-------|
| 1. Android framework CarrierConfig | (runtime `PersistableBundle`; not a static corpus file; overridable via pixel-volte-patch) | Android Telephony Framework (`CarrierConfigManager`) | Framework |
| 2. cfgdb modem NV items | `carrierconfig/` | Shannon baseband modem (provisioned by the telephony framework **[inferred]**) | Modem |
| 3. UE capability profiles (`uecapconfig`) | `uecapconfig/` | Shannon baseband modem | Modem |

**cfgdb** is a per-carrier **modem NV-item configuration** database: `cfg.db` maps SIM → carrier (`carrier_info`) → `confmap`/`confman` → confseqs; confseqs are sets of **Shannon modem NV items** (id = `zlib.crc32(NV-item-name) & 0xFFFFFFFF`, int64-array values), some CLZ4-compressed. The telephony framework reads cfgdb and provisions these NV items to the Shannon modem **[inferred]**. cfgdb is **NOT** the Android-framework `CarrierConfig` PersistableBundle system (AOSP string keys such as `carrier_volte_available_bool`); it operates at the modem layer. The exact on-device cfgdb-to-modem provisioning path has not been directly observed and is **[inferred]** throughout this document.

**uecapconfig** contains binary protobuf capability profiles advertised by the modem to the network during RRC connection establishment. **[inferred]** Selection of the appropriate profile is performed by the modem firmware, keyed by carrier identity; the selection mechanism is not directly evidenced in Part 1 and is examined in Parts 3–4.

Both cfgdb (Layer 2) and uecapconfig (Layer 3) are modem-layer configuration. The Android framework CarrierConfig (Layer 1) is a separate higher layer used by the telephony stack; it is runtime-overridable (e.g., via pixel-volte-patch) and is entirely independent of cfgdb NV items.

### 1.2 File inventory

#### carrierconfig/

| Path | Type | Role |
|------|------|------|
| `cfg.db` | SQLite database | Central index: carrier identity resolves via `confmap`/`confman_*` tables to confseqs |
| `confseqs/` | Directory (1,055 files) | Per-carrier configuration sequences (binary protobuf) |
| `manifests/` | Directory (278 files) | Per-carrier manifests; each references its confseqs by 20-byte truncated SHA-256 (first 20 bytes of the confseq's SHA-256) (no uecap ref) |
| `confseqs_symbolic_link_mapping` | Text | Maps symbolic-link names to real confseq targets |
| `manifests_symbolic_link_mapping` | Text | Maps symbolic-link names to real manifest targets |
| `build.info` | Text (INI-like) | Build timestamp, builder host, modem hash, config hash |
| `release-label` | Text | Human-readable release label for this cfgdb drop |
| `cfg.sha2` | Text | SHA-2 digest of `cfg.db` for integrity verification |

`cfg.db` row counts (full corpus):

| Table | Rows | Description |
|-------|------|-------------|
| `carrier_info` | 3,099 | Carrier records (MCC/MNC + GID/IMSI matchers) |
| `carrier_name` | 1,513 | Human-readable carrier name strings |
| `confnames` | 281 | Configuration sequence name registry |
| `confmap` | 439 | Carrier ID → confseq mappings |
| `confman_*` | 278 tables | Per-manifest config manager override tables |

#### uecapconfig/

| Path | Type | Role |
|------|------|------|
| 1,398 `*.binarypb` files (1,399 total directory entries incl. `.DS_Store`) | Binary protobuf | Per-carrier UE capability profiles |

### 1.3 Release and build lineage

The release label stored in `carrierconfig/release-label` is:

```
cfgdb-zmbu_p25-260429-B-15308590
```

The `build.info` file records (verbatim):

```
[build-info]
	date = 2026-04-29T08:11:33+00:00
	user = android-build
	uname = "Linux r-36404f8caf3535e4-ng8s 5.15.0-1013-gcp #18~20.04.1-Ubuntu SMP Sun Jul 3 08:20:07 UTC 2022 x86_64 x86_64 x86_64 GNU/Linux"
	modem = e43797a2089c274699e38d269b71beafd8619d0b
	config = 8cb8409ed4b598675468fdb4f9c3dfea38f15e89
```

The build token `260429-B-15308590` (date `260429` + build number `15308590`) appears in both the cfgdb release label and the corresponding modem radio image filename `radio-mustang-g5400i-…-260429-b-15308590.img`. **[inferred]** The matching build string strongly indicates that the cfgdb drop and the modem image were built and shipped together as a unit; `build.info` records the modem binary hash (`modem =`) so the framework can detect a mismatch. Direct confirmation would require reading the image header.

The `cfg.sha2` file (`b93504638520ebf0b406769ca6061d76e4bbd85c56c9ddedc94a25b4`) provides an integrity digest for `cfg.db` independent of the release label.

*Evidence: `analysis/inventory.py` → `analysis/out/inventory.txt`.*

## Part 2 — cfgdb (modem NV-item database)

*What this part covers: the on-device `cfgdb` database format (`cfg.db`, confseqs, manifests), the protobuf encoding of confseq and manifest records, and how carrier IDs map to modem NV-item configuration sequences.*

### 2.1 Schema

`cfg.db` contains eight fixed tables plus a family of 278 `confman_<hash>` override tables (one per carrier manifest). The fixed tables are documented below. All DDL is verbatim from the corpus.

*Evidence: `analysis/cfgdb_sqlite.py` → `analysis/out/cfgdb_sqlite.txt`.*

---

#### carrier_info (3,099 rows)

The primary SIM-identity table. Each row encodes one set of SIM attributes that resolve to a `carrier_id`.

```sql
CREATE TABLE carrier_info
             (carrier_id INTEGER NOT NULL,
              mccmnc TEXT NOT NULL CHECK (mccmnc not glob "*[^0-9]*"),
              imsi_prefix_xpattern TEXT CHECK (imsi_prefix_xpattern not glob "*[^0-9_%]*") DEFAULT "%",
              spn TEXT CHECK (spn!="") DEFAULT "%",
              plmn_name TEXT,
              gid1 TEXT CHECK (gid1 not glob "*[^0-9a-fA-F%]*") DEFAULT "%",
              gid2 TEXT CHECK (gid2 not glob "*[^0-9a-fA-F%]*") DEFAULT "%",
              preferred_apn TEXT,
              iccid_prefix TEXT CHECK (iccid_prefix not glob "*[^0-9]*"),
              privelege_access_rule TEXT)
```

Column notes:

| Column | Type | Default | CHECK constraint | Usage in corpus |
|--------|------|---------|-----------------|-----------------|
| `carrier_id` | INTEGER NOT NULL | — | — | all 3,099 rows |
| `mccmnc` | TEXT NOT NULL | — | digits only | all 3,099 rows |
| `imsi_prefix_xpattern` | TEXT | `%` | digits, `_`, `%` only | 299 rows non-wildcard |
| `spn` | TEXT | `%` | non-empty | 516 rows non-wildcard |
| `plmn_name` | TEXT | NULL | — | 0 rows (unused in this corpus) |
| `gid1` | TEXT | `%` | hex digits or `%` | 689 rows non-wildcard |
| `gid2` | TEXT | `%` | hex digits or `%` | 8 rows non-wildcard |
| `preferred_apn` | TEXT | NULL | — | 0 rows (unused in this corpus) |
| `iccid_prefix` | TEXT | NULL | digits only (no `%` permitted) | 7 rows non-wildcard |
| `privelege_access_rule` | TEXT | NULL | — | 2 rows non-wildcard |

The column name `privelege_access_rule` is preserved verbatim from the schema (misspelling of "privilege").

---

#### Carrier identity — the AOSP carrier_id registry

The `carrier_id` integer used throughout cfgdb is **identical to the AOSP TelephonyProvider `canonical_id`** defined in
[`carrier_list.textpb`](https://android.googlesource.com/platform/packages/providers/TelephonyProvider/+/refs/heads/main/assets/latest_carrier_id/carrier_list.textpb)
(committed locally as `analysis/ref/carrier_list.textpb`).
At runtime this is the value returned by `TelephonyManager.getSimCarrierId()` after SIM identification; see Step 2 of the resolution chain in §4.1 for the runtime flow.

**`carrier_info` as a SQL flattening of `carrier_attribute`**

Each AOSP `carrier_id` block contains one or more `carrier_attribute` sub-messages, each encoding one complete set of SIM attributes that identify the carrier.  cfgdb's `carrier_info` table is a direct SQL materialisation of this structure: one `carrier_info` row per `carrier_attribute` block per carrier.  The field mapping is:

| AOSP `carrier_attribute` field | cfgdb `carrier_info` column | Notes |
|---|---|---|
| `mccmnc_tuple` (repeated) | `mccmnc` | one `carrier_info` row per tuple within a block |
| `imsi_prefix_xpattern` | `imsi_prefix_xpattern` | `%` when unset (cfgdb DEFAULT) |
| `spn` | `spn` | `%` when unset |
| `gid1` | `gid1` | `%` when unset |
| `gid2` | `gid2` | `%` when unset |
| `iccid_prefix` | `iccid_prefix` | NULL when unset (no `%` in cfgdb CHECK) |

The `%` sentinel encodes "match anything" and corresponds to the absence of that refiner in the AOSP textproto.  Multiple `carrier_attribute` blocks per carrier → multiple `carrier_info` rows for the same `carrier_id`; the matching logic is logical OR (any row match suffices).

**Coverage snapshot** [verified vs `carrier_list.textpb` main @ 2026-06-29; `analysis/out/carrier_id_aosp.txt`]

| Metric | Value |
|---|---|
| AOSP `canonical_id` entries | 1,514 |
| cfgdb distinct `carrier_id` values (`carrier_name` ∪ `confnames`) | 1,517 |
| cfgdb ids present in AOSP | **1,458 / 1,517 (96.1%)** |
| Name agreement among matched ids | **1,405 / 1,455 compared (96.6%)** |
| cfgdb-only ids | **59** |

The 50 name mismatches among the 1,455 compared are upstream rebrands reflected in AOSP but not yet synced back to cfgdb; examples: T-Mobile NL (cfgdb `T-Mobile`, AOSP `Odido`), Etisalat UAE (cfgdb `Etisalat`, AOSP `e& UAE`).

Attribute spot-check for two anchor carriers:
- **AT&T (1187, `us_att`):** cfgdb `carrier_info` mccmncs match AOSP `mccmnc_tuple` sets exactly — `['310030', '310070', '310170', '310280', '310380', '310410', '310560', '310680', '310950', '311180']` (MATCH).
- **Verizon (1839, `us_vzw`):** cfgdb carries one additional mccmnc (`310004`) absent from AOSP textpb (310004 is not assigned to any carrier in `carrier_list.textpb`); all 34 AOSP-listed mccmncs are present in cfgdb.  This single addition is consistent with cfgdb being updated slightly ahead of the public registry **[inferred]**.

**Google-private extensions (cfgdb-only ids)**

The 59 cfgdb-only carrier_ids are Google-private and have no `canonical_id` in the public AOSP registry:

| Category | Count | Examples |
|---|---|---|
| id 0 — wildcard / catch-all | 1 | `wildcard` |
| id 999999 — test SIM sentinel | 1 | `TEST NINES` |
| ids 1–19,999 not in AOSP | 13 | recent carrier additions not yet synced to AOSP |
| ids ≥ 20,000 — Google-private block | 44 | `ptcrb` (20001), `eiottest` (20002), `l_testsim` (20003), `us_vzwprivate` (20006), `wildcard_5g` (20009), `jp_kddi_5gsa` (20010), `wildcard_5gsa` (20033), `RFU_A`–`RFU_J` (20012–20021) |

The ≥ 20,000 block covers PTCRB/conformance test SIMs, 5G-SA variants (`wildcard_5gsa=20033`, `jp_kddi_5gsa=20010`), Google-private MVNO configs (`us_vzwprivate=20006`, `us_att_mvno=20007`), and Reserved-For-Use placeholders (`RFU_A`–`RFU_J`).

---

#### carrier_name (1,513 rows)

Human-readable carrier display names, one per carrier_id.

```sql
CREATE TABLE carrier_name
             (carrier_id INTEGER NOT NULL UNIQUE,
              name TEXT NOT NULL)
```

---

#### carrier_parent (187 rows)

Inheritance relationships between carrier IDs. A child carrier inherits configuration from its parent unless overridden.

```sql
CREATE TABLE carrier_parent
             (carrier_id INTEGER NOT NULL UNIQUE,
              parent_id INTEGER NOT NULL)
```

---

#### confmap (439 rows)

The bridge from carrier identity to the configuration manifest. Each row maps a `carrier_id` to a manifest identified by a `sha256(content)[:40]` (40 hex chars = first 20 bytes of SHA-256).

```sql
CREATE TABLE confmap (carrier_id TEXT NOT NULL UNIQUE, parent_id TEXT, confman TEXT NOT NULL)
```

The `confman` value is simultaneously:
1. **[inferred]** The filename of the manifest binary in the `manifests/` directory.
2. The hash suffix of the corresponding `confman_<hash>` table inside `cfg.db`.

The optional `parent_id` column mirrors `carrier_parent`; it is NULL in the majority of rows in this corpus.

---

#### confnames (281 rows)

Maps numeric carrier IDs to short internal identifier strings (e.g., `us_vzw`, `us_att`).

```sql
CREATE TABLE confnames
  (carrier_id INTEGER NOT NULL UNIQUE, name TEXT NOT NULL)
```

---

#### iin (24 rows, 5 distinct carriers)

Maps ICCID Issuer Identification Numbers to carrier IDs. The `iccid_prefix` values use a trailing `%` wildcard (e.g., `8914800%`).

```sql
CREATE TABLE iin(
  "carrier_id" TEXT,
  "iccid_prefix" TEXT
)
```

---

#### regional_fallback (1 row in this corpus)

Provides a per-country fallback carrier when no other match is found. The corpus contains only the single placeholder row `('0', '0')`, suggesting this table is populated sparsely or on demand.

```sql
CREATE TABLE regional_fallback( 
  "country_code" TEXT,
  "carrier_id" TEXT
)
```

---

#### versions

Schema and release version metadata.

```sql
CREATE TABLE versions (name text, version integer)
```

In this corpus:

| name | version |
|------|---------|
| `telephony` | 117440542 |
| `ts25` | 20230410 |
| `confpack` | `cfgdb-zmbu_p25-260429-B-15308590` |

Note: the `confpack` version is a string despite the column type declared as `integer`; SQLite's type affinity rules allow this.

---

#### confman_\<hash\> family (278 tables)

One table per carrier manifest. The table name is `confman_` followed by a `sha256(content)[:40]` (40 hex chars = first 20 bytes of SHA-256) that matches the `confmap.confman` value and **[inferred]** the manifest filename in `manifests/`.

```sql
-- example (hash truncated for readability):
CREATE TABLE confman_7a7346c82b2403b061be482d8d206a34fc09b8ba (confseq TEXT NOT NULL)
```

Each row holds one `confseq` value: a `sha256(content)[:40]` (40 hex chars = first 20 bytes of SHA-256) identifying a confseq blob file in the `confseqs/` directory. A carrier's full configuration is the union of all confseq blobs listed in its manifest table. Multiple rows may share the same `confseq` hash (duplicate entries are possible; verified for Verizon's manifest).

---

#### Table relationships

```
carrier_info.carrier_id
    ├── carrier_name.carrier_id       (display name)
    ├── confnames.carrier_id          (internal name, e.g. us_vzw)
    ├── confmap.carrier_id            (→ confman hash)
    │       └── confman_<hash>.confseq (→ confseq file in confseqs/)
    └── carrier_parent.carrier_id     (→ parent carrier_id)

iin.carrier_id                        (ICCID-based lookup, bypasses carrier_info)
regional_fallback.carrier_id          (MCC-based fallback, bypasses carrier_info)
```

---

### 2.2 SIM-to-carrier matching

When the Android Telephony framework reads a SIM, it must resolve the SIM's attributes to a `carrier_id` so it can look up the correct modem NV-item configuration sequence via `confmap` and then provision those NV items to the Shannon modem **[inferred]**. The matching algorithm operates against `carrier_info`.

*Evidence: `analysis/cfgdb_sqlite.py` → `analysis/out/cfgdb_sqlite.txt`.*

#### Primary key: mccmnc

Every row in `carrier_info` carries a `mccmnc` value (all 3,099 rows have a non-wildcard value; the column is NOT NULL with a digits-only CHECK). The MCC+MNC pair read from the SIM's IMSI selects the candidate set of rows.

#### Optional refiners

If multiple rows share the same `mccmnc`, the framework narrows the match using one or more refiners:

| Refiner column | Wildcard semantics | Corpus usage |
|---|---|---|
| `imsi_prefix_xpattern` | `_` = any single digit; `%` = zero or more digits | 299 rows |
| `spn` | `%` = match any (default); non-`%` = exact SIM SPN string | 516 rows |
| `gid1` | hex digits; `%` = match any (default) | 689 rows |
| `gid2` | hex digits; `%` = match any (default) | 8 rows |
| `iccid_prefix` | exact prefix, no wildcard (CHECK enforces digits only) | 7 rows |
| `privelege_access_rule` | no constraint; 2 rows in corpus | 2 rows |

The default value for `imsi_prefix_xpattern`, `spn`, `gid1`, and `gid2` is `%` (match anything), so a row with all-wildcard refiners is a catch-all for its `mccmnc`.

**[inferred]** The framework selects the most specific matching row — i.e., the row with the greatest number of non-wildcard refiner columns satisfied by the SIM. No explicit priority column exists in `carrier_info`; the specificity ordering is inferred from AOSP `CarrierResolver` source and the column defaults.

#### Supplementary lookup tables

**iin**: When the SIM ICCID is available before the full IMSI, the `iin` table maps ICCID prefixes directly to a `carrier_id`. The 24 rows in this corpus cover 5 carriers; `iccid_prefix` values use a trailing `%` wildcard (e.g., `8914800%` for Verizon). **[inferred]** This path accelerates carrier identification when the SIM is first inserted and before full IMSI decoding.

**regional_fallback**: If no `carrier_info` row matches, the `regional_fallback` table maps a country code (derived from MCC) to a fallback `carrier_id`. In this corpus the table contains only the placeholder row `('0', '0')`, so it provides no practical fallback.

#### carrier_parent inheritance

187 rows in `carrier_parent` define a parent–child carrier hierarchy. When the framework resolves a `carrier_id` that has a `carrier_parent` entry, it also loads the parent's configuration and applies the child's overrides on top. **[inferred]** This enables MVNO specializations (child) to reuse the MNO base configuration (parent) without duplicating every confseq entry.

#### Worked example: Verizon (us_vzw, carrier_id = 1839)

```
us_vzw carrier_id = 1839
```

Verizon occupies 35 rows in `carrier_info` (query limited to 8 below for illustration):

```
mccmnc   imsi_prefix_xpattern  spn  gid1                iccid_prefix
20404    %                     %    BAE0000000000000     None
310004   %                     %    %                    None
310012   %                     %    %                    None
310590   %                     %    %                    None
310591   %                     %    %                    None
310592   %                     %    %                    None
310593   %                     %    %                    None
310594   %                     %    %                    None
```

The MCC+MNC codes `310004`, `310012`, `310590`–`310599` are Verizon domestic networks; `20404` (Netherlands KPN) with `gid1 = BAE0000000000000` is a roaming entry that matches only SIMs carrying that specific GID Level 1 value.

The `iin` table adds a further route: ICCID prefix `8914800%` maps directly to carrier_id 1839.

The `confmap` entry for carrier_id 1839 points to manifest hash `7a7346c82b2403b061be482d8d206a34fc09b8ba`. The corresponding `confman_7a7346c82b2403b061be482d8d206a34fc09b8ba` table lists 29 confseq truncated SHA-256 (`sha256(content)[:40]`) hashes, all of which resolve to files present in `confseqs/`.

---

### 2.3 Resolution chain

*Evidence: `analysis/cfgdb_reconcile.py` → `analysis/out/reconcile.txt`; `analysis/cfgdb_confseqs.py` → `analysis/out/confseqs.txt`.*

#### The chain

From a resolved `carrier_id`, the full configuration is obtained via a fixed two-step chain:

```
carrier_id
  └── confmap.confman (sha256(content)[:40] — 40 hex chars, first 20 bytes of SHA-256)
        ├── cfg.db table  confman_<hash>  (one row per confseq truncated SHA-256)
        └── manifests/<hash>              (binary protobuf, one ref per confseq truncated SHA-256)
              └── confseqs/<hash>         (plain protobuf 945; CLZ4-compressed 104; PEM cert 6)
```

The `confman` hash in `confmap` is simultaneously the SQLite table name suffix (`confman_<hash>`) and the filename under `manifests/`. Both name the same carrier; they independently list the carrier's confseqs by 20-byte truncated SHA-256 (first 20 bytes of SHA-256). The `confmap.parent_id` column is non-null for 161 of the 439 carriers; **[inferred]** the parent carrier's confseqs are inherited and the child's entries are applied on top, mirroring the `carrier_parent` hierarchy described in §2.2.

#### Named-module system

Confseq names follow a two-part convention: `<carrier-or-module-stem>.<applicability-suffix>`. The suffix encodes the context in which the configuration sequence is active:

Of the 1,055 confseq files in the corpus, 945 parse as plain protobuf and are decoded below; the remaining 110 do not parse as plain protobuf: 104 begin with magic bytes `434c5a34` (`"CLZ4"`) and are CLZ4-compressed blobs; 6 carry a PEM certificate header (`-----BEGIN CERT`). See §2.4 for details.

| Suffix | Count (of 945 decodable confseqs) | Meaning |
|--------|----------------------------------|---------|
| `.sim1` | 252 | Active when this carrier is on SIM slot 1 |
| `.sim2` | 252 | Active when this carrier is on SIM slot 2 |
| `.common` | 59 | Active regardless of SIM slot |
| `.multislot` | 111 | Active in multi-SIM / dual-standby scenarios **[inferred]** |
| (none) | 271 | No slot scoping (legacy or platform-global entries) |

The stem is typically the carrier's internal short name from `confnames` (e.g., `us_vzw`, `rogers`, `telekom_de`). A carrier's `confman_*` table usually contains separate `.sim1` and `.sim2` confseq pairs for its own configuration, so that per-slot settings (network mode, CA policy, IMS) can differ between the two SIM positions.

#### Shared functional modules

Several confseq stems are shared across many or all carriers, implementing network-technology policies that apply regardless of the specific carrier:

| Module stem | Suffixes present | Carrier coverage | Role |
|-------------|-----------------|-----------------|------|
| `endc_nr_ca_common` | `.sim1`, `.sim2`, `.common` | 439 (all) | EN-DC NR carrier-aggregation common policy |
| `endc_nr_ca_common_manual` | `.sim1`, `.sim2` | 439 (all) | Manual EN-DC NR CA override |
| `wildcard-5g` | `.sim1`, `.sim2` | 215 | 5G NR wildcard policy for broad carrier set |
| `eu_nr_common` | `.sim1`, `.sim2` | 58 | European NR common policy |
| `wildcard-5gsa` | `.sim1`, `.sim2` | 1 | 5G SA (standalone) wildcard policy |
| `rogers_5gsa` | `.sim1`, `.sim2`, `.common` | 1 | Rogers-specific 5G SA policy |

Every carrier in the corpus therefore carries the same `endc_nr_ca_common` and `endc_nr_ca_common_manual` confseqs as a universal baseline for EN-DC capability configuration.

#### Manifest flag fields

Each entry in a `manifests/<hash>` protobuf refers to a confseq by truncated SHA-256 (`sha256(content)[:40]`) and carries three additional fields. The `flag1` field takes values in `{None, 1, 2, 3, 4}` across the corpus, likely encoding SIM-slot applicability **[inferred]**. The `f4` field takes value `1` (3,892 refs) or is absent (3,209 refs). The `f8` field is `4` on every single ref in the corpus (7,101 of 7,101). The exact semantics of `f4` and `f8` are **[unverified]**; see §7.1 for the open-question analysis.

#### Reconciliation verdict (confman_* table vs manifests/\<hash\>)

The script `cfgdb_reconcile.py` reconciles, for every one of the 439 `confmap` rows, the set of confseqs listed in the corresponding `confman_*` SQLite table against the set listed in the matching `manifests/<hash>` binary file.

Results from `analysis/out/reconcile.txt` (full corpus):

```
confmap rows: 439

== reconciliation verdict (all carriers) ==
  differ            : 439
```

All 439 carriers classify as `differ`. Every one of the 20 mismatches printed by the script shows:

```
  dups=3  only_sql=0  only_man=0
```

No carrier has a missing `confman_*` table or a missing manifest file. Corpus-wide aggregates (all 439 carriers): carriers with `only_sql>0` = 0; carriers with `only_man>0` = 0; carriers with `dups>0` = 439. No carrier has any truncated SHA-256 hash present in one representation but absent from the other. The sole source of the `differ` verdict is SQL duplicate entries (`dups=3`): the confman_* table for every carrier contains exactly one confseq hash four times, yielding three duplicates. The manifest file for the same carrier contains the same hash four times as well; both representations are therefore consistent in their duplication. The duplicated entry is `09600f3f9f68a2712a82d55133a935643a1a1dd5` (multiplicity 4 in every confman_* table); this file is CLZ4-compressed (magic bytes `434c5a34`). The script correctly flags it because its de-duplication test is applied to SQL rows only.

Note: the reconciliation compares confseq hash **sets** (truncated SHA-256) and does not test whether the confseq ordering within each confman_* table or manifest is identical.

Summary: the two representations (SQLite `confman_*` table and `manifests/<hash>` binary file) are **set-identical** for all 439 carriers. The systematic 4× repetition of a single confseq entry means neither representation is strictly de-duplicated, but both are mutually consistent.

### 2.4 Wire format

*Evidence: `analysis/cfgdb_format.py` → `analysis/out/format.txt`; machine-readable schema: `cfgdb.proto`.*

#### Confseq storage forms

Of the 1,055 confseq files in the corpus, 945 are plain protobuf (surveyed below). The remaining 110 do not parse as plain protobuf:

- **104** begin with magic bytes `0x434c5a34` (`"CLZ4"`) — CLZ4-compressed blobs.
- **6** begin with `-----BEGIN CERT` (first 16 bytes: hex `2d2d2d2d2d424547494e204345525449`, ASCII `-----BEGIN CERTI`) — PEM certificate files stored in the same directory.

`cfgdb.proto` (repository root) describes the plain protobuf form. CLZ4 decompression is implemented in `analysis/cfgdb_nvitems.py` (pure Python, no new dependencies); the decompressed payload shares the same `ConfSeqData` protobuf shape (verified against all 104 CLZ4 files; see `analysis/out/nvitems.txt`).

#### Confseq protobuf structure

A confseq file is a single `ConfSeqData` message (`data` in `confseq2.proto`) with three top-level fields:

| Field | Number | Wire type | Content |
|-------|--------|-----------|---------|
| `Revision` | 1 | LEN (string) | Version string, e.g. `"v1.0"` or `"v10.0"` |
| `Name` | 2 | LEN (string) | Module name, e.g. `us_vzw.sim1` |
| `nvitem` | 4 | LEN (repeated message) | Modem NV-item entries |

Each `NvItem` message carries two fields:

- **Field 1** (`id`, `int64`): `zlib.crc32(NV-item-name) & 0xFFFFFFFF` — the NV item's identifier. See §2.5.
- **Field 2** (`item`, repeated `Val`): zero or more value wrappers; more than one value = an array.

Each `Val` message contains a single field:

- **Field 3** (`value`, repeated `int64`): the encoded value(s). A string is stored as char-code values (e.g. `gTCS_FCI_info` = `[85, 83, 65, 53, 48]` for "USA50"). See `analysis/out/nvitems.txt`.

Across all 24,216 entries in the 945 plain-protobuf confseqs, **field 3 is the only field ever observed inside a `Val` message** (29,064 occurrences; no other field number is present). This confirms that `Val` is a schema-typed wrapper: the wire type is always the same; the semantic type is externally determined.

#### Arrays-as-repeated, strings-as-char-code arrays

A scalar config value is encoded as a single `Val` in the `item` repeated field. A multi-valued (array) config value is encoded as multiple `Val` messages — 2,216 of the 24,216 entries carry more than one `Val`.

Strings are **not** encoded as UTF-8 bytes: each character is encoded as its Unicode code point in a separate element of `Val.value`. For example, NV item `gTCS_FCI_info` on `cellcom-core.sim1` encodes `"USA50"` as the five-element `value` array `[85, 83, 65, 53, 48]` (U = 85, S = 83, A = 65, 5 = 53, 0 = 48). A string value and an integer array are therefore **wire-identical** — no reader can distinguish them without out-of-band type information from the NV schema.

#### Schema-driven value typing

Because `Val.value` elements are untyped `int64`, every NV item requires external type information to be interpreted correctly:

- A scalar `1` may be a boolean `true`, the integer `1`, or the first character of a string.
- An array of scalars may be an integer array or a character-code string.

The NV schema (embedded in the modem firmware, not in the corpus files) maps each NV-item id to its declared type. The NV-item name — and hence its type — can be recovered from the g5400c NV table (60,153 entries; `~/Downloads/confseq_ext/g5400c-260519-260521_nv.csv`). See `analysis/out/nvitems.txt`.

#### Format version spread

The `Revision` field (field 1 of `ConfSeqData`) takes the following values across the 945 plain-protobuf files in this corpus:

| Version | Count |
|---------|-------|
| `v1.0` | 855 |
| `v2.0` | 37 |
| `v1.1` | 25 |
| `v3.0` | 10 |
| `v4.0` | 5 |
| `v9.0` | 5 |
| `v10.0` | 5 |
| `v1.2` | 3 |
| **Total** | **945** |

`v1.0` accounts for 855 of 945 files (90.5%) and is the predominant format in this corpus. The non-contiguous jump from `v4.0` to `v9.0` to `v10.0` suggests internal versioning increments that are not externally documented.

#### CLZ4 storage form

Of the 1,055 confseq files, 104 begin with magic bytes `0x434c5a34` (`"CLZ4"`). These are compressed using an LZ4 block payload with a 16-byte proprietary header:

```
struct CLZ4Header:          # little-endian
    magic            4s    # b'CLZ4'
    uncompressed_size I     # decompressed length in bytes
    compressed_size  I     # length of the LZ4-block payload that follows
    checksum         I     # not verified by the modem firmware
```

Decompression: `payload = data[16:16+compressed_size]`; decode LZ4 block sequences into `uncompressed_size` bytes; the result parses as a plain `ConfSeqData` protobuf. A dependency-free pure-Python implementation is in `analysis/cfgdb_nvitems.py`; it passes self-validation against all 104 CLZ4 files (see `analysis/out/nvitems.txt`).

Editing a CLZ4 confseq requires: decompress → edit NV items → recompress (LZ4 block) → re-prepend the 16-byte header → recompute `sha256(content)[:40]` for content-addressing (§2.7).

#### Manifest wire format

Each `Manifest` file in `manifests/` (278 total) is a single `Manifest` message:

| Field | Number | Wire type | Content |
|-------|--------|-----------|---------|
| `format_version` | 1 | LEN (string) | Always `"v0.1"` in this corpus (all 278 manifests) |
| `name` | 2 | LEN (string) | Manifest name string |
| `carrier_id` | 3 | VARINT (uint32) | Carrier ID |
| `refs` | 5 | LEN (repeated message) | Ordered list of confseq references |

Each `ConfSeqRef` sub-message contains:

| Field | Number | Content |
|-------|--------|---------|
| `content_hash` | 2 | first 20 bytes of SHA-256 (`sha256(content)[:20]`) of the referenced confseq; used as the filename under `confseqs/` and as the value in `confman_*` SQLite rows |
| `flag1` | 1 | Optional uint32; values `{1, 2, 3, 4}` or absent; likely encodes SIM-slot applicability **[inferred]** |
| `f4` | 4 | Present on 3,892 of 7,101 refs, always value `1` when present; meaning **[unverified]** |
| `f8` | 8 | Present on all 7,101 refs, always value `4`; meaning **[unverified]** |

The `content_hash` field (field 2 of `ConfSeqRef`) is the join key between the manifest binary and both the `confseqs/` file tree and the `confman_<hash>` SQLite table rows. The manifest format version `v0.1` is distinct from the confseq format version family (`v1.0`–`v10.0`); the two numbering spaces are independent.

### 2.5 NV item ID: CRACKED **[established]**

*Evidence: `analysis/cfgdb_nvitems.py` → `analysis/out/nvitems.txt`; NV table `~/Downloads/confseq_ext/g5400c-260519-260521_nv.csv` (60,153 rows); `~/Downloads/confseq_ext/confseq2.proto` (authoritative schema).*

#### The field is a modem NV-item identifier, not an Android key hash

The integer previously labelled `key_hash` in field 1 of each NV item is a **modem NV-item identifier**: `id = zlib.crc32(NV-item-name) & 0xFFFFFFFF`. This was established by cross-referencing the corpus against the g5400c Samsung Shannon modem NV table (60,153 named items):

- **crc32 verify**: `zlib.crc32(name.encode('utf-8')) & 0xFFFFFFFF` equals the table's `crc32` column for all 60,153 rows (0 mismatches) **[established]**.
- **Full coverage**: all 2,658 distinct IDs in the 945 plain-protobuf confseqs map to a named NV item in that table (100%) **[established]**.
- **CLZ4 coverage**: after LZ4 decompression, all CLZ4 confseq IDs are also 100% covered.
- **Example**: id `922505959` = `TCS_GV_OPT_CARRIER_TYPE`; id `287538830` = `gTCS_FCI_info` (values `[85, 83, 65, 53, 48]` = "USA50" on `cellcom-core.sim1`); others resolve to `NASL3.*`, `!SAEL3.*`, `PSS.*`, `HCOMMON.*`, `TCS_GV_*` namespaces.

#### Why the earlier key-hash sweep found nothing

`cfgdb_keyhash.py` tested `crc32` (and 7 other hash functions) over **AOSP `CarrierConfigManager` key names** (e.g. `carrier_volte_available_bool`). CRC32 was the correct function — but applied to the wrong namespace. The actual inputs are **modem NV-item names** like `TCS_GV_OPT_CARRIER_TYPE`, not Android framework key strings. There was no match because the candidate names were entirely wrong, not because the hash function was wrong.

#### Interface and Playbook A consequence

The `id → NV-item-name` mapping is now complete for 60,153 items. A tool wanting to decode confseqs symbolically uses: `crc_to_name = {int(r['crc32']): r['item'] for r in csv.DictReader(open(nv_csv))}`.

This changes the picture for **Playbook A**:

- The **surgical** route is **no longer blocked by missing key names**: NV-item names are now recoverable. However, the NV items are **modem NV items**, not Android `CarrierConfigManager` keys — the target modem parameters for IMS enablement (e.g. VoLTE, VoWiFi) must be identified from the NV item namespace (e.g. search for `IMS`, `VOLTE`, `WFC`, `VoLTE` in the NV item names). Editing the correct NV items in the target carrier's confseq is now feasible.
- The **coarse** route (clone a confseq bundle from an IMS-capable carrier) is unaffected and remains the lower-risk option.

---

### 2.6 Config assembly

*Evidence: `analysis/cfgdb_integrity.py` → `analysis/out/integrity.txt`; corroborating: `analysis/out/cfgdb_sqlite.txt`, `analysis/out/format.txt`, `analysis/out/reconcile.txt`. All assembly semantics below are **[inferred]** from static corpus analysis; on-device runtime behaviour has not been observed.*

#### Step 1 — confmap look-up

Given a resolved `carrier_id` (via §2.2), the runtime looks up `confmap` to retrieve:
- **`confman`** — the hex identifier of the carrier's configuration bundle. This value is simultaneously: (a) the name suffix of a `confman_<hash>` SQLite table inside `cfg.db`, and (b) the filename of the corresponding binary manifest proto in `manifests/`. All 278 distinct confman hashes map to both a matching table and a matching manifest file (verified by reconciliation, §2.3).
- **`parent_id`** — the `carrier_id` of a parent carrier, present for 161 of 439 confmap rows.

#### Step 2 — confseq list from the confman table

The `confman_<hash>` table lists confseq hashes in insertion (`rowid`) order. The rowid sequence is the **assembly order**: confseqs are applied from rowid 1 upward. Within a NV-item id namespace, the last confseq applied wins (last-write / highest-rowid override semantics) **[inferred]**.

Because confman tables and manifests are set-identical per carrier (§2.3), both representations encode the same confseq set. The confman table rowid order is the authoritative assembly sequence for the SQLite-based runtime path.

#### Step 3 — SIM-slot filtering

Each confseq carries a name suffix that encodes which SIM slot it applies to (format.txt, confseqs.txt):

| Suffix | Count | Scope |
|---|---|---|
| `.sim1` | 252 | SIM slot 1 only |
| `.sim2` | 252 | SIM slot 2 only |
| `.common` | 59 | both slots |
| `.multislot` | 111 | multi-slot contexts |
| (none) | 271 | bare carrier or module name |

The runtime selects only the confseqs applicable to the active SIM slot before merging; `.common` and bare-name sequences are applied regardless of slot **[inferred]**.

#### Parent inheritance

`confmap.parent_id` records a parent carrier for 161 of 439 rows. However, in all 161 parent-child pairs the child references the **same** confman table as its parent (`confmap parent-child pairs: 161; share same confman table: 161` — integrity.txt). At the confseq-bundle level, `confmap.parent_id` therefore produces no distinct override layer; child and parent receive byte-for-byte identical confseq sets. Its purpose at the confseq-assembly level is therefore **[unverified]** — likely metadata for runtime carrier matching rather than confseq differentiation.

The separate `carrier_parent` table (187 rows, cfgdb_sqlite.txt) also maps `carrier_id → parent_id` for a wider set of carriers. Its relationship to confseq assembly is **[unverified]**; it may serve a fallback in SIM-to-carrier resolution (§2.2) rather than confseq selection.

#### Shared confman tables

The 278 distinct confman tables serve 439 carriers; up to 91 carriers share a single confman table (`max carriers sharing one confman table: 91` — integrity.txt). Carriers sharing a table receive identical confseq bundles. Modifying a shared confman table affects all carriers that reference it.

#### What the corpus cannot prove

The exact runtime merge semantics (order of slot filtering vs override application, timing of parent fallback, handling of the CLZ4 confseqs present in every bundle via the 4× duplicated CLZ4 entry) require on-device observation or source-code review. The manifest `flag1` field (values 1/2/3/4/None, §2.4) likely encodes per-slot scope but the mapping is **[unverified]**. The semantics of `f4` (present in 3,892 of 7,101 refs) and `f8` (value `4` on all 7,101 refs) are **[unverified]**.

---

### 2.7 Integrity model

*Evidence: `analysis/cfgdb_integrity.py` → `analysis/out/integrity.txt`.*

#### Correction: content-addressing uses sha256[:40], not SHA-1

Earlier documentation (including §2.3 reconciliation notes and the §2.5 text, now corrected) described confseq and manifest references as "SHA-1". The corpus shows this is wrong. `cfgdb_integrity.py` tested `SHA-1(content) == filename` across all 1,055 confseq files and found **1,055 mismatches** (0 matches). Testing `sha256(content)[:40] == filename` — the first 160 bits (40 hex chars) of SHA-256 — yields **1,055 matches** (0 mismatches), confirmed verbatim in integrity.txt:

```
confseq files checked: 1055; SHA-1(content) != filename: 1055
confseq files checked: 1055; sha256(content)[:40] == filename: 1055
```

The same scheme applies to manifest files:

```
manifest files checked: 278; sha256(content)[:40] == filename: 278
```

The manifest proto field decoded as `sha1` in `lib.py` (field 2 of the ref message) therefore stores a 20-byte value that is the first 20 bytes of SHA-256, not a SHA-1 digest. The field name in the proto is a misnomer as seen from the corpus.

This applies to all confseq storage forms — plain protobuf (945 files), CLZ4-compressed (104 files), and PEM-cert (6 files). Content-addressing operates on raw bytes regardless of payload encoding.

#### Rule 1 — confseq content-addressing

A confseq filename is `sha256(raw_bytes)[:40]`. Editing any byte changes the hash and therefore the filename. Any valid edit must:

1. Produce the new payload bytes.
2. Compute `new_hash = sha256(new_bytes)[:40]`.
3. Write `confseqs/<new_hash>` to the confseqs directory.
4. `UPDATE confman_<table_hash> SET confseq = '<new_hash>' WHERE confseq = '<old_hash>'` in `cfg.db`.
5. In `manifests/<table_hash>`: replace the 20-byte raw value of field 2 in the matching ref message with `bytes.fromhex(new_hash)`.

Because modifying the manifest bytes changes its sha256[:40] (Rule 2 below), steps 4–5 must be done together; see "Minimal valid-edit procedure" below.

**Shared-table consequence:** up to 91 carriers share one confman table. Changing a confseq hash in a shared table silently changes the bundle for every carrier that references it. To change a confseq for one carrier only, the confman table must be cloned first (creating a new carrier-specific table with a new hash and a new manifest).

#### Rule 2 — manifest content-addressing and table-name consistency

Manifest filenames are `sha256(manifest_bytes)[:40]`, and the confman table name is that same hash. Modifying a manifest's binary content (e.g., updating a ref field) produces a new hash and therefore requires:

- A new manifest file under the new hash name.
- Renaming `confman_<old_hash>` to `confman_<new_hash>` in `cfg.db`.
- Updating `confmap.confman` for every carrier that referenced `<old_hash>`.

#### Rule 3 — cfg.sha2 relationship

`cfg.sha2` = `b93504638520ebf0b406769ca6061d76e4bbd85c56c9ddedc94a25b4` (56 hex chars = 224 bits). `cfgdb_integrity.py` tested sha224, sha256, and sha512_224 of the current `cfg.db`:

```
sha224(cfg.db)    = 53924fee37ed8e52db138a78af8f3213adcfd8186ee8de846fb157aa
sha256(cfg.db)    = 399f5c581a7bffcbc01b26550b54f8436bcb5019f37e21f22599f0b7b434655c
sha512_224(cfg.db)= 23b2d413042d37a982e3ba811df6a482b78138dea38b2ac8d7460ac0
```

**None match.** The relationship between `cfg.sha2` and `cfg.db` is **[unverified]**. The most likely explanation is that sha224(cfg.db) or sha256(cfg.db)[:56] was computed at Android build time and the SQLite file has since been page-reorganised by WAL checkpointing. Editing `cfg.db` will change its hash regardless; since the on-disk value is already unverifiable, cfg.sha2 cannot be updated reliably without on-device testing.

#### Rule 4 — versions / release-label consistency

The `versions` table (integrity.txt):

```
versions table: [('telephony', 117440542), ('ts25', 20230410), ('confpack', 'cfgdb-zmbu_p25-260429-B-15308590')]
```

The `confpack` value matches `release-label` (`cfgdb-zmbu_p25-260429-B-15308590`). Any version-checking code in the telephony stack that compares `versions.confpack` against `release-label` will detect a mismatch if only one is updated. Both must be updated together.

#### Minimal valid-edit procedure

To replace one confseq for a target carrier without affecting other carriers (plain-protobuf form; for CLZ4 confseqs, decompress first — see §2.4 CLZ4 storage form):

1. **Read** the existing confseq: `old_bytes = open('confseqs/<old_hash>', 'rb').read()`.
2. **Produce** the new payload `new_bytes`.
3. **Compute** `new_hash = hashlib.sha256(new_bytes).hexdigest()[:40]`.
4. **Write** `confseqs/<new_hash>` with `new_bytes`.
5. **Clone the confman table** if the carrier shares a confman table with others (up to 91 carriers share one): copy `confman_<old_manifest_hash>` to `confman_<cloned_hash>` (where `<cloned_hash>` is `<old_manifest_hash>` — the manifest has not yet changed) and `UPDATE confmap SET confman = '<cloned_hash>' WHERE carrier_id = '<target_id>'`.
6. **Update the cloned confman table row**: `UPDATE confman_<cloned_hash> SET confseq = '<new_hash>' WHERE confseq = '<old_hash>'` (operates on the cloned table, not the shared original).
7. **Update the manifest proto**: replace the 20-byte ref field 2 value for the old confseq with `bytes.fromhex(new_hash)`; write the new manifest bytes to a temporary buffer.
8. **Compute new manifest hash**: `new_manifest_hash = sha256(new_manifest_bytes)[:40]`.
9. **Write** `manifests/<new_manifest_hash>` with `new_manifest_bytes`.
10. **Rename the confman table**: in `cfg.db`, create `confman_<new_manifest_hash>` with the updated confseq list, then drop `confman_<old_manifest_hash>`.
11. **Update confmap**: `UPDATE confmap SET confman = '<new_manifest_hash>' WHERE carrier_id = '<target_id>'`.
12. **Optionally update** `versions.confpack` and `release-label` to a new build label.

`cfg.sha2` cannot be re-computed reliably ([unverified] relationship); leave it or update with on-device testing.

---

## Part 3 — uecapconfig

*What this part covers: the `uecapconfig` directory layout, the filename-selector scheme, capability tiers and profiles, `lte_*` fallback files, `ap_plmn_mapping` legend, the protobuf payload structure and band-encoding, and the evolution from the single-file-per-carrier Pixel 7 era to the current 16-profile scheme.*

*Authority: `~/Projects/code/pixel-uecaps-toolbox` (source + README); schema: `~/Projects/code/pixel-uecaps-toolbox/proto/ue_caps.proto`. For read/inspect/edit/package operations see the toolbox command reference in the README. Standalone filename decoder (no build required): `~/Projects/reference/uecaps_info.py`.*

*Evidence: `analysis/uecap_summary.py` → `analysis/out/uecap.txt`.*

### 3.1 Directory layout

The `uecapconfig/` directory (data path: `lib.data_dir("ue")`) contains 1,398 `.binarypb` files of three kinds:

| Kind | File pattern | Count | Role |
|------|-------------|-------|------|
| Carrier capability profiles | `<CARRIER>_<NUMBER>.binarypb` | 1,389 | Per-carrier × per-SKU UE capability profiles |
| LTE-only fallback | `lte_<NUMBER>.binarypb` | 8 (`analysis/out/uecap.txt`: `lte_*.binarypb files: 8`) | Shannon-hardware-selected LTE-only profiles |
| PLMN legend | `ap_plmn_mapping.binarypb` | 1 | Maps each PLMN to a carrier config name |

The 1,389 carrier files span **89 distinct carriers** (toolbox `check`: `files: 1398 | carriers: 89 | legend entries: 80`). `analysis/uecap_summary.py` reports 90 using a simplified name-extraction heuristic that groups the 8 `lte_*` files under the pseudo-carrier `lte`; the toolbox count of 89 (which excludes `lte_*` files) is the authoritative figure.

*To audit the full directory:*
```bash
pixel-uecaps-toolbox check ~/Projects/reference/uecapconfig
```

### 3.2 Filename selector

The trailing `NUMBER` in `<CARRIER>_<NUMBER>.binarypb` is **not** a hash or version — it is a selector key:

```
NUMBER = carrier-signature × SKU-profile-prime
```

- **Carrier signature**: the greatest common divisor of all `NUMBER` values for a given carrier across its full set of files. It is an opaque integer embedding the carrier's identity; all sibling files share it as a factor.
- **SKU-profile anchor prime**: a unique prime that divides `NUMBER` for exactly one file per carrier. A Pixel device loads the file whose `NUMBER` is divisible by the device SKU's anchor prime — so **which file gets picked depends on the exact Pixel SKU**.

There are **16 SKU capability profiles** (P01–P16). The full table is encoded in `uecaps_info.py` (`PROFILES` list):

| Profile | Anchor prime | Full prime tag | Family |
|---------|-------------|----------------|--------|
| P01 | 167 | 67 · 167 | A |
| P02 | 1847 | 83 · 1847 | B |
| P03 | 8969 | 233 · 281 · 8969 | A |
| P04 | 688679 | 331 · 688679 | A |
| P05 | 224309 | 293 · 224309 | B |
| P06 | 196911437 | 196911437 | A |
| P07 | 3616442437 | 3616442437 | A |
| P08 | 66813533 | 66813533 | B |
| P09 | 1176929627 | 1176929627 | B |
| P10 | 154921957 | 154921957 | B |
| P11 | 3347 | 193 · 3347 | A |
| P12 | 1002739 | 97 · 1002739 | A |
| P13 | 6791 | 509 · 6791 | B |
| P14 | 1334093 | 3209 · 1334093 | B |
| P15 | 2912407 | 2912407 | A |
| P16 | 3539 | 89 · 1013 · 3539 | A |

**Worked example** (from toolbox README): `VZW_193698151252893.binarypb` — NUMBER 193698151252893 = 3⁵ · 7² · 17 · 67 · 167 · 85523. The anchor prime is 167 (P01, Pixel 10 Pro Fold); the carrier signature is 85523 (GCD of all 16 VZW file numbers).

**Selection rule [inferred]:** the modem firmware identifies the device's SKU profile tag and loads the unique carrier file whose NUMBER is divisible by that tag. The anchor-prime scheme guarantees uniqueness: no two profiles share an anchor. Direct observation of the modem's file-selection logic has not been performed.

*To see the full SKU math for a specific file:*
```bash
pixel-uecaps-toolbox inspect --full VZW_193698151252893.binarypb
```
*To decode any filename standalone (no Rust build required):*
```bash
python3 ~/Projects/reference/uecaps_info.py VZW_193698151252893.binarypb
```
*To get the carrier × profile matrix as CSV:*
```bash
pixel-uecaps-toolbox matrix ~/Projects/reference/uecapconfig
```

### 3.3 Capability tiers, families, and reference stubs

#### Tiers and fingerprints

The in-file capability **fingerprint** (protobuf field 1 of `UeCaps`, typed `uint64` in `proto/ue_caps.proto`) identifies both the tier and the capability family. There are four valid fingerprint values. `analysis/uecap_summary.py` reads field 1 from every non-legend `.binarypb` file and counts occurrences (`analysis/out/uecap.txt`: 874888686 → 652 files, 862505271 → 507 files, 707802847 → 119 files, 627223094 → 119 files; total 1397 = 1398 − 1 legend). The family/tier mapping is from `uecaps_info.py` `FP_INFO` (and identically in toolbox `src/model.rs` `fp_info`).

| Fingerprint (field 1) | Family | Tier | Profiles present | Count in corpus |
|-----------------------|--------|------|-----------------|----------------|
| 874888686 | A | main | P01–P16 (all 16) | 652 |
| 862505271 | B | main | P01–P16 (all 16) | 507 |
| 707802847 | A | alt | P01–P14 (no P15/P16) | 119 |
| 627223094 | B | alt | P01–P14 (no P15/P16) | 119 |

**Main tier** (16 profiles, fingerprints 874888686/862505271): US, European, and APAC major carriers.

**Alt tier** (14 profiles, fingerprints 707802847/627223094): India and emerging-market carriers. Profiles P15 (anchor 2912407) and P16 (anchor 3539) are absent from the alt tier.

The 17 alt-tier carriers in this corpus (toolbox `check`): AIRTEL, DT_NL, EU_COMMON1, EU_GENERIC_3CA, KPN, OTHERS, RJIO, SPRINT, TELENOR_DK, TELENOR_SE, TEST_FIELD, VF_CZ, VF_IN, VF_RO, VF_TR, VI_IN, VODA_IDEA.

#### Reference stubs

**224 files** (16 alt-tier carriers × 14 profiles) are **reference stubs** (toolbox `check`: `224 files`). A stub carries the fingerprint (field 1) but contains no capability payload (field 3 `combo_groups` is absent). From `proto/ue_caps.proto`:

```protobuf
message UeCaps {
  uint64 version = 1; // capability fingerprint → family/tier
  optional int32 id = 2;
  repeated ComboGroup combo_groups = 3; // absent in a stub → no capability payload
  ...
  uint64 unknown = 9; // stub delegation reference
}
```

Stubs carry a non-zero `unknown` (field 9) value that **[inferred]** references the `EU_COMMON1` carrier config — the single alt-tier carrier that ships real capability payloads. All 16 other alt-tier operators ship stubs and delegate to it.

*To confirm whether a file is a stub:*
```bash
pixel-uecaps-toolbox inspect AIRTEL_<NUMBER>.binarypb
# Output: "reference stub — no capability payload"
```

### 3.4 lte_* fallback files

The 8 `lte_<NUMBER>.binarypb` files are LTE-only carrier-aggregation profiles. They sit outside the 16-profile SKU scheme: no profile anchor prime divides their numbers, and they carry no NR combo payload.

**Selection mechanism**: `lte_*` files are selected by the Shannon modem based on a hardware category code burned into the modem firmware, **not** by SIM or MCC. The `LteCaps` / `LteCombo` / `LteComponent` message types in `proto/ue_caps.proto` define the wire format. From `toolbox inspect lte_844857560.binarypb`:

```
LTE config : sta5_na
             modem-selected by hardware category 0x812 (Shannon g5400), not SIM/MCC
```

The full hardware-category-to-file mapping (all 8 `lte_*` IDs with their category codes) is in toolbox `src/model.rs` `LTE_CONFIGS`.

*To inspect an LTE fallback:*
```bash
pixel-uecaps-toolbox inspect lte_844857560.binarypb
# Shows: LTE config name, hardware category, and LTE CA combo list
pixel-uecaps-toolbox inspect --full lte_844857560.binarypb
# Adds per-CC DL class·MIMO / UL class and bcs
```

### 3.5 ap_plmn_mapping — the PLMN legend

`ap_plmn_mapping.binarypb` maps each mobile network (PLMN, encoded as 3GPP packed-BCD integers) to a carrier config name. This corpus contains **80 legend entries** (toolbox: `legend entries: 80`). The carrier names in the legend are identical to the `<CARRIER>` prefixes on the capability files.

Six carriers appear in the capability-file tree but are **absent from the legend** in this corpus (DT_NL, KPN, OTHERS, PLATFORM, TELENOR_DK, TELENOR_SE), confirmed by toolbox `check`.

Schema (from `proto/ue_caps.proto`):
```protobuf
message PlmnMap { repeated Carrier carriers = 1; }
message Carrier {
  repeated uint64 plmns = 1 [packed = false]; // 3GPP packed-BCD; unpacked for bit-for-bit identity
  uint64 index = 2;  // internal index
  string name = 3;   // carrier-config name == <CARRIER> filename prefix
}
```

*To read or edit the legend:*
```bash
# Decode to editable TOML
pixel-uecaps-toolbox mapping decode < ap_plmn_mapping.binarypb > mapping.toml
# Re-encode after editing
pixel-uecaps-toolbox mapping encode < mapping.toml > new_mapping.binarypb
# One-shot: append a PLMN to a carrier
pixel-uecaps-toolbox mapping inject-plmn VZW 250-99 \
    < ap_plmn_mapping.binarypb > new_mapping.binarypb
```

### 3.6 Capability payload: combo groups and band encoding

#### Payload structure

A carrier capability file (`UeCaps`, `proto/ue_caps.proto`) has three payload sections:

| Field | Type | Role |
|-------|------|------|
| `combo_groups` (field 3) | `repeated ComboGroup` | Band combination groups (the main CA/NR capability payload) |
| `dl_feature_per_cc_list` (field 6) | `repeated ShannonFeatureSetDlPerCCNr` | NR downlink per-CC feature sets |
| `ul_feature_per_cc_list` (field 7) | `repeated ShannonFeatureSetUlPerCCNr` | NR uplink per-CC feature sets |

Each `ComboGroup` contains a header (`combo_header`, with BCS and power-class fields) and a list of `Nested2` combos (`Nested1` and `Nested2` are the literal inner message names in `proto/ue_caps.proto`). Each `Nested2` combo holds one or more `ComboFeatures` component carriers; each `ComboFeatures` entry identifies the band via the integer field `band`.

#### Band encoding — LTE = id, NR = 10000 + id

The `band` integer in `ComboFeatures` follows a two-range convention:

| Range | Meaning | Encoding |
|-------|---------|---------|
| `band < 10000` | LTE band | raw value = 3GPP LTE band number |
| `band ≥ 10000` | NR band | raw value = 10000 + 3GPP NR band number |

**Evidence** (`analysis/out/uecap.txt`, VZW file `VZW_132493905285110.binarypb`):

```
LTE bands (id): [2, 5, 13, 48, 66]
NR bands (id-10000): [2, 5, 66, 77, 260, 261]
```

The raw field values for LTE were 2, 5, 13, 48, 66 (equal to the LTE band IDs directly). The raw field values for NR were 10002, 10005, 10066, 10077, 10260, 10261; subtracting 10000 yields the 3GPP NR band numbers 2, 5, 66, 77, 260, 261. The mmWave bands 260 and 261 (raw IDs 10260, 10261) confirm the encoding holds across the full numeric range, not only sub-6 GHz bands.

#### Shannon NR per-CC feature sets

`ShannonFeatureSetDlPerCCNr` and `ShannonFeatureSetUlPerCCNr` encode per-component-carrier NR capabilities in a Shannon-proprietary format:

| Field | Role |
|-------|------|
| `max_scs` | Maximum subcarrier spacing (kHz) |
| `max_mimo` | Maximum MIMO layers |
| `max_bw` | Maximum bandwidth (MHz) |
| `max_mod_order` | Maximum modulation order |
| `bw_90mhz_supported` | 90 MHz bandwidth support |
| `max_mimo_non_cb` (UL only) | Maximum non-codebook MIMO layers |

Each `ComboFeatures.dl_feature_index` / `ul_feature_index` indexes into the file-level `dl_feature_per_cc_list` / `ul_feature_per_cc_list` tables, allowing combos to share feature-set entries.

*To view per-CC capability details:*
```bash
pixel-uecaps-toolbox inspect --full VZW_193698151252893.binarypb
# Per-band: DL BW/MIMO/QAM/SCS/90MHz and UL cb/nonCb MIMO, modulation
pixel-uecaps-toolbox inspect --toml VZW_193698151252893.binarypb
# Machine-readable TOML with all structured fields
```

### 3.7 cheetah → mustang evolution

The Pixel 7 era corpus (`uecapconfig-cheetah/`) shipped **one file per carrier** with no numeric suffix: `VZW.binarypb`, `ATT.binarypb`, etc. The cheetah corpus is exactly **89 files** (`analysis/out/uecap.txt`: `cheetah .binarypb files: 89; cheetah naming: one-per-carrier (no numeric suffix) — confirmed`) — one per carrier, no profile scheme.

The current "mustang" corpus (`uecapconfig/`) expands that to the **16-profile selector scheme**: each carrier ships up to 16 files distinguished by the `NUMBER` suffix, enabling the modem to load a different capability profile depending on the exact Pixel SKU (sub-6 GHz vs mmWave, differing RF front-end configurations). The total grows from 89 (cheetah, one file per carrier) to 1,398 (mustang, 89 carriers × up to 16 profiles each; alt-tier carriers top out at 14).

The cheetah files use the same `UeCaps` protobuf schema; the wire format is consistent across generations. The addition of the `NUMBER` suffix is a naming-layer change atop the same binary encoding.

## Part 4 — Connection & likely modem algorithm

*What this part covers: the runtime join between cfgdb and uecapconfig — the PLMN bridge that links the two systems, the division of responsibility between framework policy and modem capability, and the likely end-to-end algorithm executed at SIM insertion and RRC registration.*

*Evidence: `analysis/correspondence.py` → `analysis/out/correspondence.csv`.*

### 4.1 The PLMN bridge

Both cfgdb and uecapconfig are indexed by the same cellular identifier: the PLMN (Public Land Mobile Network code, the concatenation of MCC and MNC). cfgdb carries PLMN in `carrier_info.mccmnc` (plain decimal string, e.g., `310480`). uecapconfig carries PLMN in `ap_plmn_mapping.binarypb`, encoded as 3GPP packed-BCD integers decoded by nibble order `(mcc1, mcc2, mcc3, mnc3, mnc1, mnc2)` with a `0xF` sentinel in the `mnc3` nibble for 2-digit MNCs. PLMN is therefore the natural join key between the two systems.

`analysis/correspondence.py` proves this join corpus-wide: for each `(mccmnc, cfgdb_slug)` row in `carrier_info JOIN confnames`, the script looks up the PLMN in the decoded `ap_plmn_mapping` and accumulates the count of shared PLMNs per `(cfgdb_slug, uecap_carrier)` pair, producing `analysis/out/correspondence.csv`.

**Join results** (`analysis/out/correspondence.csv`):

| Metric | Value |
|--------|-------|
| Total (cfgdb_slug, uecap_carrier) pairs | 179 |
| Distinct cfgdb slugs with a match | 165 of 281 |
| Distinct uecap carrier names matched | 61 of 80 |
| cfgdb slugs with no uecap correspondence | 116 |
| cfgdb slugs matching more than one uecap carrier | 12 |
| Maximum shared PLMNs in one pair (`de_vf` ↔ `VF_DE`) | 177 |

*Row denominators sourced from prior tasks: `confnames` 281 (`analysis/out/inventory.txt`, `analysis/out/cfgdb_sqlite.txt`); uecap legend 80 (§3.5).*

The 116 cfgdb slugs with no correspondence are carriers whose PLMNs do not appear in `ap_plmn_mapping` — generally smaller regional operators or markets for which Google does not ship a dedicated Shannon capability profile. The 12 multi-match slugs are MVNOs or roaming carriers whose PLMNs straddle two or more host-network profiles (e.g., `us_dish` matches both `DISH` and `TELUS`; `zz_truphone` matches `KPN_NL`, `O2_UK`, and `VF_NL`).

#### Full correspondence table

*Source: `analysis/out/correspondence.csv`. The `us_vzw ↔ VZW` row is **bold**.*

| cfgdb\_slug | uecap\_carrier | shared\_plmns |
|------------|---------------|---------------|
| at\_a1 | EU\_COMMON | 4 |
| at\_a1mpn | EU\_COMMON | 1 |
| at\_h3g | EU\_COMMON | 3 |
| at\_h3g\_fi | EU\_COMMON | 1 |
| at\_help | EU\_COMMON | 1 |
| at\_spusu | EU\_COMMON | 1 |
| at\_tchibo | EU\_COMMON | 1 |
| at\_tmobile | DT\_DE | 7 |
| au\_optus | OPTUS | 2 |
| au\_telstra | TELSTRA | 3 |
| au\_vf | VHA | 1 |
| be\_orange | ORANGE\_BE | 1 |
| bh\_zain | APAC\_COMMON | 1 |
| ca\_bell | BELL | 3 |
| ca\_cloudcore | TELUS | 2 |
| ca\_fizz | VIDEOTRON | 1 |
| ca\_freedom | FREEDOM | 1 |
| ca\_koodo | TELUS | 2 |
| ca\_publicmobile | TELUS | 2 |
| ca\_rogers | ROGERS | 4 |
| ca\_rogers\_5gsa | ROGERS | 1 |
| ca\_sasktel | SASKTEL | 4 |
| ca\_sasktel | VF\_NL | 1 |
| ca\_tbaytel | ROGERS | 2 |
| ca\_telus | TELUS | 2 |
| ca\_videotron | VIDEOTRON | 4 |
| ch\_spusu | EU\_COMMON | 1 |
| cz\_tmobile | DT\_DE | 1 |
| de\_1and1 | 1\_1\_DE | 1 |
| de\_1and1 | O2\_DE | 5 |
| de\_1and1 | ORANGE\_FR | 1 |
| de\_alditalk | O2\_DE | 2 |
| de\_dtag | DT\_DE | 26 |
| de\_o2 | O2\_DE | 2 |
| de\_spusu | EU\_COMMON | 1 |
| de\_tchibo | O2\_DE | 1 |
| de\_vf | VF\_DE | 177 |
| dk\_h3g | EU\_COMMON | 1 |
| dk\_tdc | EU\_COMMON | 2 |
| dk\_telenor | TELENOR\_NO | 2 |
| dk\_telia | TELIA\_DK | 2 |
| eiottest | TEST\_LAB | 3 |
| es\_digi | MOVISTAR\_ES | 1 |
| es\_jazztel | ORANGE\_FR | 2 |
| es\_o2 | MOVISTAR\_ES | 2 |
| es\_orange | ORANGE\_FR | 2 |
| es\_simyo | ORANGE\_FR | 1 |
| es\_vf | VF\_ES | 6 |
| fr\_bouygues | BOUYGUES | 3 |
| fr\_bouyguesb2b | BOUYGUES | 3 |
| fr\_coriolis | ORANGE\_FR | 1 |
| fr\_coriolis | SFR | 1 |
| fr\_free | FREE\_FR | 1 |
| fr\_orange | ORANGE\_FR | 4 |
| fr\_sfr | SFR | 3 |
| gb\_spusu | EU\_COMMON | 1 |
| gr\_telekom | DT\_DE | 3 |
| hk\_h3g | APAC\_COMMON | 1 |
| hk\_smartone | APAC\_COMMON | 1 |
| hr\_telekom | DT\_DE | 18 |
| hu\_telekom | DT\_DE | 1 |
| ie\_h3g | 3\_IE | 1 |
| ie\_vf | VF\_IE | 3 |
| il\_hotmobile | VF\_NL | 2 |
| in\_airtel | IN\_GEN | 23 |
| in\_bsnl | IN\_GEN | 21 |
| in\_idea | IN\_GEN2 | 22 |
| in\_rjio | IN\_GEN | 22 |
| in\_vf | IN\_GEN2 | 46 |
| it\_coopvoce | TIM\_IT | 2 |
| it\_iliad | EU\_COMMON | 2 |
| it\_iliad | FREE\_FR | 1 |
| it\_kena | TIM\_IT | 1 |
| it\_spusu | EU\_COMMON | 1 |
| it\_tim | TIM\_IT | 1 |
| it\_vf | VF\_IT | 3 |
| it\_vianova | TIM\_IT | 3 |
| it\_windtre | WINDTRE | 2 |
| jp\_dcm | DCM | 2 |
| jp\_kddi | KDDI | 6 |
| jp\_kddi\_5gsa | KDDI | 1 |
| jp\_rakuten | ORANGE\_FR | 1 |
| jp\_rakuten | RAKUTEN | 1 |
| jp\_sbm | SBM | 1 |
| kr\_kt | APAC\_COMMON | 1 |
| kr\_lgu | APAC\_COMMON | 1 |
| kr\_skt | APAC\_COMMON | 1 |
| lu\_post | EU\_COMMON | 1 |
| me\_telekom | DT\_DE | 1 |
| mk\_telekom | DT\_DE | 1 |
| mx\_att | MX\_COMMON | 8 |
| mx\_movistar | MX\_COMMON | 2 |
| mx\_telcel | MX\_COMMON | 1 |
| nl\_enreach | KPN\_NL | 1 |
| nl\_enreach | VF\_NL | 1 |
| nl\_kpn | KPN\_NL | 3 |
| nl\_tmo | TMO\_NL | 2 |
| nl\_vf | VF\_NL | 25 |
| no\_ice | EU\_COMMON | 1 |
| no\_naf | TELIA\_NO | 1 |
| no\_onecall | TELIA\_NO | 1 |
| no\_telenor | TELENOR\_NO | 2 |
| no\_telia | TELIA\_NO | 1 |
| nz\_spark | APAC\_COMMON | 1 |
| pl\_telekom | DT\_DE | 1 |
| pt\_meo | EU\_COMMON | 2 |
| pt\_nos | EU\_COMMON | 1 |
| pt\_vf | EU\_COMMON | 3 |
| ro\_telekom | DT\_DE | 9 |
| sa\_stc | EU\_COMMON | 1 |
| sa\_zain | EU\_COMMON | 1 |
| se\_h3g | EU\_COMMON | 1 |
| se\_telavox | TELENOR\_NO | 2 |
| se\_tele2 | EU\_COMMON | 1 |
| se\_telenor | TELENOR\_NO | 1 |
| se\_telia | TELIA\_SE | 1 |
| se\_teliab2b | TELIA\_SE | 2 |
| se\_vimla | TELENOR\_NO | 1 |
| sg\_m1 | M1 | 1 |
| sg\_simba | APAC\_COMMON | 1 |
| sg\_singtel | SINGTEL | 1 |
| sg\_starhub | STARHUB | 1 |
| sk\_telekom | DT\_DE | 1 |
| tr\_telekom | EU\_COMMON | 1 |
| tr\_vf | VF\_DE | 1 |
| tw\_apt | FET | 2 |
| tw\_cht | CHT | 1 |
| tw\_fet | FET | 2 |
| tw\_tstar | TWM | 1 |
| tw\_twm | TWM | 1 |
| uk\_asda | VF\_UK | 2 |
| uk\_ee | EE | 4 |
| uk\_esimgo | VF\_UK | 2 |
| uk\_gamma | 3\_UK | 1 |
| uk\_gamma | EU\_COMMON | 1 |
| uk\_gigs | VF\_UK | 2 |
| uk\_h3g | 3\_UK | 3 |
| uk\_id | 3\_UK | 1 |
| uk\_leb | VF\_UK | 1 |
| uk\_o2 | O2\_UK | 1 |
| uk\_sky | EU\_COMMON | 2 |
| uk\_smarty | 3\_UK | 1 |
| uk\_tesco | O2\_UK | 4 |
| uk\_tkm | VF\_UK | 2 |
| uk\_vf | VF\_UK | 3 |
| uk\_virgin | EE | 3 |
| us\_att | ATT | 8 |
| us\_att\_bootstrap | ATT | 1 |
| us\_att\_mvno | ATT | 5 |
| us\_bluegrass | VZW | 1 |
| us\_cbrs | GOOGLE\_COMCAST\_ | 1 |
| us\_cbrs\_chatr | GOOGLE\_COMCAST\_ | 2 |
| us\_cellcom\_core | CELLCOM | 2 |
| us\_cox | VZW | 2 |
| us\_cricket | ATT | 1 |
| us\_cspire | CSPIRE | 1 |
| us\_cspire | VF\_NL | 1 |
| us\_dish | DISH | 13 |
| us\_dish | TELUS | 1 |
| us\_firstnet | ATT | 3 |
| us\_firstnet\_pacific | ATT | 1 |
| us\_firstnet\_samoa | ATT | 1 |
| us\_spectrum | VZW | 3 |
| us\_tmo | TMO | 14 |
| us\_tmo\_fi | TMO | 3 |
| us\_tmo\_mvno\_ultra | TMO | 22 |
| us\_tmo\_private | TMO | 1 |
| us\_tracfone | VZW | 3 |
| us\_uscc | USC | 2 |
| us\_uscc | VF\_NL | 2 |
| us\_uscc\_fi | USC | 1 |
| us\_visible | VZW | 2 |
| us\_vzw | VF\_NL | 1 |
| **us\_vzw** | **VZW** | **8** |
| us\_vzwprivate | VZWPRIVATE\_US | 1 |
| us\_xfinity | VZW | 4 |
| zz\_truphone | KPN\_NL | 2 |
| zz\_truphone | O2\_UK | 2 |
| zz\_truphone | VF\_NL | 3 |

The table shows that major US carrier pairs (`us_att ↔ ATT`: 8; **`us_vzw ↔ VZW`: 8**; `us_tmo ↔ TMO`: 14) share substantial PLMN sets, confirming that the join is not coincidental. MVNO slugs such as `us_tracfone`, `us_xfinity`, `us_visible`, and `us_bluegrass` also resolve to `VZW`, reflecting the Verizon-hosted MVNO PLMN range.

---

### 4.2 Three-layer model and division of responsibility

The three configuration layers have distinct, non-overlapping roles:

**Layer 1 — Android framework CarrierConfig (pixel-volte-patch layer):** PersistableBundle of AOSP string keys (`carrier_volte_available_bool`, `carrier_wfc_ims_available_bool`, etc.) consumed by the Android telephony stack at the Java framework level. This layer governs whether the framework presents IMS services to the user and signals IMS intent to the modem driver layer. It is runtime-overridable (pixel-volte-patch operates here). This layer is independent of cfgdb; cfgdb edits do NOT modify framework CarrierConfig and vice versa.

**Layer 2 — cfgdb modem NV items:** confseqs are sets of Shannon modem NV items — low-level modem configuration parameters in the `TCS_GV_*`, `PSS.AIMS.*`, `NASL3.*`, `!SAEL3.*`, `HCOMMON.*`, and `OMC.*` namespaces. The telephony framework reads cfgdb and provisions these NV items to the modem **[inferred]**. The NV items govern modem-internal behaviour: IMS stack parameters, SIP timers, VoLTE/VoNR capability flags, EN-DC/NR-CA policy, SRVCC settings, and more. The shared modules `endc_nr_ca_common` and `endc_nr_ca_common_manual` (present across all 439 carriers; `analysis/out/confseqs.txt`) define the baseline EN-DC / NR carrier-aggregation NV-item policy. Additional modules such as `wildcard-5g` (215 carriers) extend NR policy for a broad carrier set.

**Layer 3 — uecapconfig (UE capability profiles):** `.binarypb` profiles describe which bands, carrier-aggregation combos, and NR feature sets the modem is willing to advertise to the network via `UECapabilityInformation`. The profile is selected per-SKU (by the divisibility selector) and per-serving-carrier (via `ap_plmn_mapping`). The profile carries no policy flags: it encodes only physical-layer capability (bands, BCS, MIMO, modulation, BW, SCS).

**Reconciling pixel-volte-patch:** the pixel-volte-patch technique modifies Layer 1 (Android framework CarrierConfig PersistableBundle), NOT Layer 2 (cfgdb NV items). Setting `carrier_volte_available_bool = true` via a pixel-volte-patch tells the Android telephony stack to attempt IMS; it does not change any Shannon modem NV item in cfgdb. Correspondingly, editing cfgdb NV items (Layer 2) does not affect the framework-layer feature flags. The two mechanisms are complementary: the framework layer governs whether IMS is signalled to the user and framework, while the modem NV items govern the modem's internal IMS stack behaviour. **[inferred]** Both layers may need to be consistent for IMS to function end-to-end.

**A band requires agreement from Layers 2 and 3**: cfgdb NV items must permit NR/EN-DC for the serving carrier **[inferred]**, and the uecap profile for that carrier must include the band in its combo list. If NV items in cfgdb disable EN-DC or NR for a carrier, the modem will not attempt those modes regardless of what it has advertised. If the uecap profile omits a band, the network cannot schedule it regardless of NV-item policy.

---

### 4.3 Likely end-to-end algorithm

The steps below are individually labelled by evidential basis. **Framework path steps (1–2, 4–5)** are grounded in the telephony framework / `CarrierConfigLoader` (AOSP), 3GPP TS 24.008 / 36.331 / 38.331, and direct corpus observation; they are marked `[established from AOSP + corpus]`. Step 3 (carrier-parent inheritance) is `[inferred]` — the AOSP `carrier_parent` table exists in the corpus but the runtime substitution path has not been directly observed. **Step 5b (cfgdb→modem provisioning)** is `[inferred]` — the exact on-device mechanism has not been directly observed. **Modem path steps (6–8)** have not been directly observed in firmware and are marked `[inferred]`.

#### Framework path — executed by Android TelephonyManager / `CarrierConfigLoader`

**Step 1 — SIM insertion, PLMN and IMSI read.** **[established from AOSP + corpus]**
The baseband reports the inserted SIM's IMSI and serving PLMN (MCC+MNC, read from EFIMSI / EFPL on the SIM card) to `TelephonyManager`. This is a 3GPP-specified interface (TS 24.008).

**Step 2 — carrier_info match → carrier_id.** **[established from AOSP + corpus]**
The framework queries `carrier_info` in `cfg.db` for a row whose `mccmnc` matches the PLMN and whose optional columns (`imsi_prefix_xpattern`, `spn`, `gid1`, `gid2`, `iccid_prefix`) further match the SIM attributes. The corpus has 3,099 `carrier_info` rows (`analysis/out/inventory.txt`, `analysis/out/cfgdb_sqlite.txt`), of which MCCMNC is non-wildcard in all 3,099; the more-specific columns narrow the match for MVNOs sharing a PLMN with their host network. The `iin` table adds an ICCID-prefix fast path (24 rows, 5 carriers, `analysis/out/cfgdb_sqlite.txt`; e.g., ICCID prefix `8914800%` → carrier_id 1839 = `us_vzw`). The result is a numeric `carrier_id`.

**Step 3 — carrier_parent resolution.**
If `carrier_parent` contains an entry for the resolved `carrier_id`, the parent `carrier_id` is substituted for configuration lookup. **[inferred]** This implements inheritance so that an MVNO child can override its host-network parent's confseqs selectively.

**Step 4 — confmap → confman table → confseq list.** **[established from AOSP + corpus]**
`confmap` maps `carrier_id` → `confman` hash → `confman_<hash>` table in `cfg.db`. The table rows each carry one confseq identifier (a `sha256(content)[:40]` truncated hash). The 278 distinct confman tables serve 439 confmap entries; up to 91 carriers share a single confman table, meaning they receive an identical confseq bundle.

**Step 5 — confseq loading.** **[established from AOSP + corpus]**
For each confseq hash, the framework loads the corresponding blob from `confseqs/`. The 1,055 confseq files break down into 945 plain protobuf, 104 CLZ4-compressed, and 6 PEM-format certificate payloads. Each confseq is a set of Shannon modem NV items (`ConfSeqData.nvitem[]`; §2.4), not Android framework CarrierConfig keys.

**Step 5b — cfgdb NV-item provisioning to modem.** **[inferred]**
The telephony framework provisions the decoded NV items to the Shannon modem. The NV items belong to modem namespaces such as `TCS_GV_*`, `PSS.AIMS.*`, `NASL3.*`, `!SAEL3.*`, `HCOMMON.*`, and `OMC.*` (see §2.5); their identifiers are `id = zlib.crc32(NV-item-name) & 0xFFFFFFFF` **[established]**. The NV-item policy modules `endc_nr_ca_common` and `endc_nr_ca_common_manual` (present across all 439 carriers; §2.3) configure the modem's EN-DC/NR-CA behaviour for every carrier. The exact on-device provisioning path (API, timing, modem sideloading mechanism) is **[inferred]** and has not been directly observed from the static corpus.

#### Modem path — executed by the Shannon baseband

**Step 6 — SKU profile selection (device boot, not SIM-dependent).**
**[inferred]** On power-up, the Shannon modem determines the device's capability profile anchor prime (P01–P16, table in §3.2). For each carrier in the `uecapconfig/` directory, the modem selects the file whose `NUMBER` is divisible by that anchor prime. The anchor-prime uniqueness guarantee (no two profiles share an anchor) means exactly one file is selected per carrier per SKU. The `lte_*` files are selected separately by hardware category code (`0x812` for the g5400 modem; §3.4), not by SKU anchor.

**Step 7 — serving PLMN → carrier name via ap_plmn_mapping.**
**[inferred]** Once the modem camps on a cell, it reads the serving cell's PLMN from the broadcast System Information Block (SIB1). It looks up this PLMN in `ap_plmn_mapping.binarypb` (80 PLMN-to-carrier-name entries; §3.5, sourced from toolbox `check` output) to obtain the carrier config name (e.g., `VZW`, `ATT`, `TMO`). If the PLMN is absent from the legend, the modem's fallback behaviour is **[unverified]**; candidate behaviours are loading `EU_COMMON1` (the alt-tier stub delegate), the generic `lte_*` profile, or no uecap profile at all.

**Step 8 — capability profile load → UECapabilityInformation.**
**[inferred]** The modem loads the pre-selected `<CARRIER>_<NUMBER>.binarypb` file for the matched carrier. This file contains the `UeCaps` protobuf: `combo_groups` (LTE and NR band combination lists), `dl_feature_per_cc_list`, and `ul_feature_per_cc_list` (NR per-CC feature sets; §3.6). During RRC connection establishment, the modem emits `UECapabilityInformation` (3GPP TS 36.331 §5.6.6 for EUTRA; 3GPP TS 38.331 §5.6.6 for NR) populated from this profile. The network uses this message to determine which bands and CA combos it may schedule for the device. Band integers below 10000 are LTE band IDs; integers ≥ 10000 are NR band IDs minus 10000 (§3.6).

#### Convergence — cfgdb policy ∩ uecap capability

**Step 9 — both modem-layer systems must allow a band.**
For the network to schedule a given band or CA combo, two independent conditions must hold simultaneously:

1. **cfgdb NV items allow it** **[inferred]**: the `endc_nr_ca_common` (and optionally `wildcard-5g`, `wildcard-5gsa`, carrier-specific NR modules) confseqs provisioned in Step 5b must not block EN-DC or NR-SA for the serving carrier via their modem NV items.
2. **uecap advertises it**: the `UECapabilityInformation` emitted in Step 8 must include the band/combo in the combo groups for the serving carrier's profile.

If cfgdb NV items disable EN-DC or NR for a carrier, the modem will not attempt those modes even though it has advertised NR combos. Conversely, if the uecap profile for a carrier omits a band (or the entire carrier is absent from `ap_plmn_mapping`), the modem will not advertise it and the network cannot schedule it, regardless of NV-item policy. Enabling a new band for a carrier that currently has neither cfgdb NV-item permission nor a uecap entry therefore requires modifying both modem-layer systems in coordination. The Android framework CarrierConfig (Layer 1, §4.2) is a third gating factor at the framework level but is independent of Layers 2 and 3.

The PLMN bridge (§4.1) shows that both layers use the same PLMN to identify the carrier, so the same serving-cell PLMN drives both lookups simultaneously.

## Part 5 — Playbook A: enable IMS for an unsupported carrier

*What this part covers: a static-analysis feasibility verdict and step-by-step approach for modifying cfgdb to enable IMS on a carrier that is not natively supported. Live on-device validation is out of scope; this part evaluates three gates — granularity, key-crack, and deployment/integrity — against the corpus evidence from Parts 1–4.*

*Evidence: `analysis/playbook_a.py` → `analysis/out/playbook_a.txt`; corroborating: `analysis/out/keyhash.txt`, `analysis/out/integrity.txt`, `analysis/out/confseqs.txt`.*

---

### 5.1 Round-trip proof (editing prerequisite)

Before any confseq edit can be committed back to the cfgdb package, the edit pipeline must be able to emit a correctly-named confseq file — i.e., one whose filename equals `sha256(new_bytes)[:40]` (the content-addressing scheme established in §2.7). `playbook_a.py` provides a structured round-trip proof:

For every plain-protobuf confseq in the corpus, `playbook_a.py` deserialises the binary into a typed model (version string, name string, and an ordered list of `(id:int, [value:int|None, ...])` NV-item tuples), re-serialises that model byte-for-byte using `encode_confseq`, and checks that `sha256(rebuilt)[:40]` equals the filename.

Result (`analysis/out/playbook_a.txt`):

```
structured round-trip (plain confseqs): ok=945 bad=0; skipped (CLZ4/cert, not plain protobuf)=110
```

All 945 plain-protobuf confseqs rebuild to their exact bytes and their reconstructed sha256[:40] matches the filename. This proves that any edit to a plain-protobuf confseq can be serialised back to a valid, correctly-named file using the `encode_confseq` routine in `playbook_a.py`.

The 110 skipped files — 104 CLZ4-compressed (`"CLZ4"` magic bytes `434c5a34`; §2.4) and 6 PEM certificate payloads — are not plain protobuf. CLZ4 confseqs are now decodable (pure-Python LZ4 in `analysis/cfgdb_nvitems.py`; §2.4 CLZ4 storage form); editing one requires: decompress → edit NV items → recompress (LZ4 block) → re-prepend header → recompute sha256[:40] (§2.7). PEM certificate payloads require re-signing and remain out of scope.

---

### 5.2 Identify target and donor carriers

#### Choosing a target

A target carrier is any carrier in the corpus for which IMS (VoLTE/VoWiFi) is not currently working. Determining this definitively requires on-device testing. For planning purposes, any carrier whose `confnames` entry is not a major IMS-capable MNO is a plausible target. MVNOs that inherit configuration from an IMS-capable host MNO via `carrier_parent` (187 rows; §2.2) may already receive IMS settings through that mechanism [inferred]. The NV-item id→name mapping is now complete (§2.5), so IMS-related NV items can be identified by name in the carrier's confseq (see §5.3.2).

#### Choosing a donor

The IMS donor should be a carrier known to have IMS working, with a rich confseq bundle that is likely to contain IMS/VoLTE/VoWiFi configuration keys. `playbook_a.py` ranks carriers by total config entry count across all their confseqs (`analysis/out/playbook_a.txt`):

```
carriers with the most config entries (candidate IMS donors):
  default                 : 2680 entries
  endc_nr_ca_common       : 1430 entries
  KDDI                    : 940 entries
  Softbank                : 882 entries
  att                     : 840 entries
  vzw                     : 836 entries
  cricket                 : 760 entries
  tmo                     : 758 entries
  cellcom-core            : 646 entries
  uscc                    : 633 entries
```

`default` and `endc_nr_ca_common` are shared functional modules (present across all 439 carriers; §2.3), not carrier-specific donors. The top carrier-specific candidates are `KDDI` (940 entries), `Softbank` (882), `att` (840), and `vzw` (836). All four are major MNOs with confirmed IMS deployments. `vzw` (Verizon, carrier_id 1839) is a well-documented IMS carrier in the AOSP framework and has the largest confseq bundle among US carriers in this corpus.

**Shared modules present** (`playbook_a.txt`): `endc_nr_ca_common: yes`, `eu_nr_common: yes`, `wildcard-5g: yes`, `wildcard-5gsa: yes`. IMS configuration is carried within carrier-core confseqs — there is no separate `ims` or `volte` named module; IMS-related modem NV items (e.g., `TCS_GV_SHANNON_VOLTE`, `PSS.AIMS.*`, `OMC.BASED.VONR.PLMN.ENABLE`) are interspersed among general modem NV items in each carrier's `.sim1`/`.sim2`/`.common` confseqs.

---

### 5.3 Three routes: framework, coarse cfgdb, surgical cfgdb

#### 5.3.0 Framework route: runtime CarrierConfig override via pixel-volte-patch (proven, recommended)

The simplest and most direct route to enabling IMS for a carrier operates at **Layer 1** (§4.2) — the Android framework CarrierConfig — and requires **no cfgdb edits at all**.

The pixel-volte-patch technique overrides the Android telephony framework's per-carrier `PersistableBundle` at runtime, setting AOSP framework keys such as `carrier_volte_available_bool = true`, `carrier_wfc_ims_available_bool = true`, and related flags. This modification targets the framework layer only; it does not touch cfgdb (Layer 2) or uecapconfig (Layer 3). Because the framework layer is runtime-overridable and cfgdb-agnostic, this route avoids the content-addressing, manifest integrity, and on-device delivery challenges of cfgdb modification entirely.

This is the **recommended first approach**. It has been proven on Pixel devices. The cfgdb routes below (§5.3.1 and §5.3.2) are relevant if the framework route alone is insufficient (e.g., if the modem NV items also need to be aligned) or if direct modem NV-item control is needed.

---

#### 5.3.1 Coarse cfgdb route: clone the donor bundle or re-point the SIM match (viable)

The coarse route works at confman/confseq-bundle granularity. It does not require identifying any individual key by name. Two variants:

**Variant A — confseq-bundle clone**: copy the donor carrier's `confman_*` table rows (or a subset of its confseq hashes) into a new confman table for the target carrier, updating `confmap` to point the target at the new table. The target carrier then receives the donor's confseq bundle and inherits the donor's IMS settings. The risk is that the bundle also carries APN, roaming, and other settings from the donor, which may need to be stripped or the target's own non-IMS confseqs added back.

**Variant B — re-point the SIM match**: add a row to `carrier_info` that matches the target SIM's MCCMNC (and any required refiners) to the donor's `carrier_id`, or insert a `carrier_parent` row that makes the target carrier inherit from the donor. This requires no confseq file edits — only `cfg.db` table changes. The risk is that the target SIM then receives the donor's full policy (APN, roaming, VoLTE provisioning state) rather than a narrowly scoped IMS enablement.

In both variants:
- **No key-hash cracking is required.** The operation is purely on confseq-bundle hashes and `confmap`/`confman_*` rows.
- The confman table and manifest content-addressing rules (§2.7 Rules 1–2) still apply: any changes to `confman_*` rows change the manifest content, which changes the manifest hash, which requires a new manifest file and a renamed `confman_<new_hash>` table (§2.7 minimal valid-edit procedure).
- **Shared-table caution**: up to 91 carriers share one confman table (§2.6). Modifying a shared table silently alters every carrier that references it. To affect only the target carrier, clone its confman table first (create `confman_<new_hash>` from the existing table, update `confmap` to point only the target carrier at the new hash).

#### 5.3.2 Surgical cfgdb route: edit individual modem NV items (UNBLOCKED)

The surgical route edits specific modem NV items in the target carrier's confseq — for example, setting an IMS-related NV item such as `TCS_GV_SHANNON_VOLTE` to an enable value (**[unverified]** which value) or adjusting `PSS.AIMS.*` IMS stack parameters. Since §2.5 established that NV-item ids are `zlib.crc32(NV-item-name) & 0xFFFFFFFF` with 100% coverage against the g5400c NV table (60,153 entries), the id→name mapping is complete and this route is **no longer blocked by missing identifiers**.

**Why the earlier key-hash sweep was irrelevant:** the cfgdb_keyhash.py sweep tested AOSP `CarrierConfigManager` key strings (e.g., `carrier_volte_available_bool`). Those are Layer-1 Android framework keys that have no presence in cfgdb. cfgdb confseqs contain modem NV items with names like `TCS_GV_SHANNON_VOLTE` — an entirely different namespace. The "block" no longer applies.

**IMS-candidate modem NV items (by name-search):** the following NV items from the g5400c NV table (60,153 entries) contain VoLTE/VoNR/VoWiFi keywords and are **present in corpus confseqs** (id = crc32 of name). Which specific items to set — and to what values — to enable IMS for a given carrier is **[unverified]**; these are candidates identified by name only.

| NV-item name | crc32 id | Notes |
|---|---|---|
| `TCS_GV_SHANNON_VOLTE` | 1228968624 | Likely Shannon modem VoLTE master switch |
| `DS_TCS_GV_SHANNON_VOLTE` | 2557641007 | Dual-SIM variant |
| `UECAPA_REL15_IMS_VONR_FR1_SUPPORT` | 3645552966 | IMS VoNR FR1 capability |
| `OMC.BASED.VONR.PLMN.ENABLE` | 867143358 | VoNR PLMN-based enablement |
| `PSS.AIMS.MO.Timer.EPSFB.VoNR.Support` | 304838890 | MO EPS-FB VoNR support |
| `PSS.AIMS.MT.Timer.EPSFB.VoNR.Support` | 520610774 | MT EPS-FB VoNR support |
| `!LTENR.NSA.VOLTE.LOW.PWR.MODE` | 2506098785 | LTE-NR NSA VoLTE low-power mode |
| `PSS.AIMS.SRVCC` | 28471430 | SRVCC (voice handover LTE→legacy) |
| `PSS.AIMS.Is.VoLTEVoWiFiSilentRetry` | 1589495017 | VoLTE/VoWiFi silent retry |

*Every row above is an **[unverified]** candidate identified by modem-NV name-search only; presence in a confseq does not imply that setting it enables IMS, nor are the required values known from static analysis.*

In addition, the following NV items are in the NV table but do **not** appear in any corpus confseq (they use modem defaults and are not explicitly set for any carrier): `NASU.VOLTE.CAPA` (2532028937), `PSS.AIMS.VONR_CAPABILITY_INFO` (927674915), `!SAEL3.VOLTE_CERTI` (1710922850).

**Surgical editing procedure:** find IMS-related NV items in the target carrier's confseq (search by crc32 id); change or add entries using the crc32(name) formula; for CLZ4 confseqs, decompress first (§2.4); recompute `sha256(content)[:40]` and update confman/manifest per §2.7. The round-trip proof (§5.1, `ok=945 bad=0`) confirms the serialisation pipeline is correct for plain-protobuf confseqs. The cfgdb-edit delivery to the device is still **[unverified]** (§5.5/Gate 3).

---

### 5.4 Re-encode per Task-7 integrity rules

Any confseq edit (coarse or surgical) must follow the content-addressing chain established in §2.7. The round-trip proof in §5.1 confirms that plain-protobuf confseqs can be correctly re-serialised; the workflow is:

1. **Deserialise** the source confseq using `model_of` (from `playbook_a.py`): extract `(version, name, entries)` where entries are `(id:int, [value:int|None, ...])` NV-item tuples. CLZ4/PEM confseqs cannot use this path (see §5.1).
2. **Modify** the typed model as required (for the coarse route, this typically means no confseq content changes — the confseq files are used as-is from the donor; only the confman tables and manifests change).
3. **Re-serialise** using `encode_confseq` and **compute** `new_hash = sha256(new_bytes).hexdigest()[:40]`.
4. **Write** `confseqs/<new_hash>`.
5. **Update** `confman_<manifest_hash>` table row: `SET confseq = '<new_hash>' WHERE confseq = '<old_hash>'`.
6. **Update** the manifest proto: replace the 20-byte `content_hash` field (field 2 of the ref) with `bytes.fromhex(new_hash)`.
7. **Recompute** `new_manifest_hash = sha256(new_manifest_bytes).hexdigest()[:40]`, write `manifests/<new_manifest_hash>`, rename the confman table to `confman_<new_manifest_hash>`, and update `confmap.confman`.
8. **Optionally** update `versions.confpack` and `release-label` together (§2.7 Rule 4); mismatching the two will cause any version-check in the telephony stack to fail.
9. **cfg.sha2**: this 56-hex-char (224-bit) value in the cfgdb package does not match any standard digest of the shipped `cfg.db` (sha224, sha256, sha512_224 all mismatch; §2.7 Rule 3). Its on-device enforcement is **[unverified]**. Leave it unchanged or update it with on-device testing; there is no reliably correct value calculable from the static corpus.

For the coarse bundle-clone variant (Variant A, §5.3.1), steps 2–4 are skipped because donor confseq files are used verbatim; only the confman tables and manifests are changed (steps 5–9).

---

### 5.5 Deployment investigation

The cfgdb package (the directory containing `cfg.db`, `confseqs/`, `manifests/`, `cfg.sha2`, and `build.info`) must be delivered to the device for the telephony framework to pick up the edited configuration. The standard path on Pixel devices is a GMS module update delivered via Play Services. An alternative is a Magisk overlay: mounting a modified version of the cfgdb directory at `/vendor/firmware/carrierconfig` (the confirmed on-device path) using Magisk's `systemless` mechanism.

Both the overlay approach and the re-verification behaviour are **[unverified]**:

- **Magisk overlay path [established]**: the on-device cfgdb path is `/vendor/firmware/carrierconfig`. Whether the telephony framework reloads carrier configuration after a runtime Magisk overlay mount (versus caching at boot) has not been observed on device [unverified].
- **cfg.sha2 enforcement** [unverified]: §2.7 Rule 3 established that `cfg.sha2` does not match any standard digest of the current `cfg.db`. If the framework verifies cfgdb integrity at load time using `cfg.sha2`, the shipped package is already in a state that would fail such a check — or the framework does not enforce it. The check's existence, algorithm, and enforcement are unknown from static analysis alone.
- **Framework signature checks** [unverified]: if cfgdb is signed (e.g., an APK or APEX module), modifying it offline would invalidate the signature. The corpus files are raw data (not an APK), so this applies to the package that wraps them, not the files directly.

Recommended investigation steps (all deferred to on-device validation, out of scope here): confirm Magisk overlay at `/vendor/firmware/carrierconfig` is accepted by the telephony framework; observe whether `cfg.sha2` causes a load failure; confirm the framework reloads on overlay application without a reboot.

---

### 5.6 Verify and rollback

**Verify**: after a deployment, check that:
1. The Android telephony stack reports the target carrier as IMS-registered (e.g., via `adb shell dumpsys telephony.registry` or `adb shell service call phone 117`).
2. A VoLTE/VoWiFi call completes successfully.
3. No regression on other carriers that share the same confman table.

**Rollback**: restore the original cfgdb package. Because confseq files are content-addressed (`sha256(content)[:40]`), the original files are not overwritten — new hash filenames are added. Rolling back consists of restoring the original `cfg.db` (with the original `confmap` and `confman_*` rows) and optionally removing the new confseq/manifest files. A Magisk overlay rollback removes the overlay module and reboots.

---

### 5.7 Feasibility verdict

The feasibility assessment is against three gates, evaluated per route.

#### Gate 1 — Granularity: can the desired change be expressed at the available edit granularity?

**Verdict: PASS (all cfgdb routes)**

- **Coarse** (§5.3.1): operates at confman/confseq-bundle granularity. No individual NV-item id needs to be known. The 945 plain-protobuf confseqs all round-trip byte-for-byte (`ok=945 bad=0; playbook_a.txt`).
- **Surgical** (§5.3.2): now also PASS — NV-item ids are `crc32(name)` (100% coverage; §2.5), so individual NV items can be targeted by name.
- **Framework route** (§5.3.0): not applicable — operates outside cfgdb entirely.

#### Gate 2 — NV-item identification: are the target modem NV-item names recoverable?

**Verdict: RESOLVED [established]**

The NV-item id is `zlib.crc32(NV-item-name) & 0xFFFFFFFF`. The g5400c NV table (60,153 entries) gives 100% coverage of all 2,831 distinct ids in the corpus (plain + CLZ4; §2.5). IMS-candidate NV-item names are identified by keyword search (§5.3.2). This gate was previously listed as BLOCKED because the sweep tested AOSP framework key names instead of modem NV-item names — an incorrect namespace; the underlying algorithm (crc32) was always correct.

#### Gate 3 — Deployment / integrity: can a modified cfgdb package be loaded by the device?

**Verdict: framework route PROVEN; cfgdb-edit routes [unverified]**

- **Framework route** (§5.3.0): proven on Pixel devices (pixel-volte-patch). No cfgdb delivery required.
- **cfgdb-edit routes** (§5.3.1 and §5.3.2): [unverified]. The static corpus analysis cannot determine whether a Magisk overlay of the cfgdb package is accepted by the telephony framework, whether `cfg.sha2` is enforced at load time (it already mismatches the shipped `cfg.db`; §2.7 Rule 3), or whether any other integrity check gates the modified package. This gate requires on-device evidence to resolve.

#### Summary

| Gate | Framework route | Coarse cfgdb | Surgical cfgdb |
|------|----------------|--------------|----------------|
| Granularity | N/A | PASS | PASS |
| NV-item id crack (crc32 + NV table) | Not required | Not required | RESOLVED [established] |
| Deployment / integrity | PROVEN | [unverified] | [unverified] |

**Overall static-analysis verdict**: the **framework route** (§5.3.0) is the recommended first approach — it is proven, cfgdb-agnostic, and avoids all cfgdb integrity/delivery concerns. For direct modem NV-item control, both cfgdb routes are now unblocked at the editing level; the sole remaining blocker is on-device delivery ([unverified]) which requires live validation out of scope for this static analysis.

---

## Part 6 — Playbook B: author band / CA-combo capability profiles

*What this part covers: step-by-step guide for creating or modifying a UE capability profile to add band and CA-combo entries to a uecapconfig file, packaging it for deployment, and the gap analysis for what the toolbox cannot do.*

*Authority: `~/Projects/code/pixel-uecaps-toolbox` README and command reference. Schema: `~/Projects/code/pixel-uecaps-toolbox/proto/ue_caps.proto` (not duplicated here; referenced by path). Evidence: `analysis/out/playbook_b.txt` (toolbox patch round-trip transcript).*

---

### 6.1 Toolbox round-trip evidence

The following transcript was captured from a live run of the toolbox against the corpus files and written to `analysis/out/playbook_b.txt`. It demonstrates the `patch create` → `patch show` → `patch filter` pipeline on real carrier data.

**Files used**:
- Base: `ATT_100936302644210.binarypb` — ATT, SKU profile 154921957 (Pixel 9 Pro XL mmWave), family B, main tier; 1490 band combos.
- Target: `VZW_132493905285110.binarypb` — VZW, SKU profile 154921957, family B, main tier; 1240 band combos.

Both files are from the same SKU profile (154921957), making them a valid `patch create` pair.

```
=== cargo build ===
Finished release profile [optimized]

=== patch create ATT_100936302644210 -> VZW_132493905285110 ===

=== patch show (head -20) ===
Combo patch (nr) · format v1
  delete 1194 · set 956 (956 add, 0 change)

deletes
  B12A + B2A↓ + B2A↓ + B30A↓ + n260(7)
  B12A + B2A↓ + B2A↓ + B30A↓ + n260(8)/(7)
  B12A + B2A↓ + B2A↓ + B30A↓ + n260(9)/(7)
  B12A + B2A↓ + B2A↓ + B30A↓ + n66A
  B12A + B2A↓ + B2A↓ + B30A↓ + n77A
  B12A + B2A↓ + B2A↓ + B66A↓ + n260(7)
  B12A + B2A↓ + B2A↓ + B66A↓ + n260(8)/(7)
  B12A + B2A↓ + B2A↓ + B66A↓ + n260(9)/(7)
  B12A + B2A↓ + B2A↓ + B66A↓ + n30A
  B12A + B2A↓ + B2A↓ + B66A↓ + n66A
  B12A + B2A↓ + B2A↓ + B66A↓ + n77A
  B12A + B2A↓ + B2A↓ + n260(7)
  B12A + B2A↓ + B2A↓ + n260(8)/(7)
  B12A + B2A↓ + B2A↓ + n260(9)/(7)
  B12A + B2A↓ + B2A↓ + n30A
  B12A + B2A↓ + B2A↓ + n66A

=== patch filter include n77 (head -5) ===
kind = "nr"
version = 1
delete = [
    "B12A + B2A↓ + B2A↓ + B30A↓ + n77A",
    "B12A + B2A↓ + B2A↓ + B66A↓ + n77A",
```

*Source: `analysis/out/playbook_b.txt` (excerpt: the `patch create` → `show` → `filter` portion). The 1490 and 1240 band-combo counts cited above come from the two `=== inspect … ===` sections that precede this excerpt in the same file.*

**Read**: `patch create` computed a diff from ATT to VZW: the patch deletes 1194 ATT-only combos and sets 956 VZW combos not in ATT (all additions; no capability changes). `patch show` renders the patch in human-readable form. `patch filter include n77` produces a new patch containing only the combos involving n77, written as a TOML patch with `kind = "nr"`. The toolbox binary is at `~/Projects/code/pixel-uecaps-toolbox` (built with `cargo build --release`; custom `target-dir` at `/private/tmp/target` per `~/.cargo/config.toml`).

---

### 6.2 Playbook

#### Step 1 — Pick a carrier and Pixel model code

Choose:

- **Target carrier**: the carrier whose uecap profile you want to change (e.g., `VZW`). This must already have files in the uecapconfig directory, because the toolbox edits existing files rather than generating them from scratch.
- **Pixel model code**: the 5-character Google model code for the target device (e.g., `G2YBB` = Pixel 9 mmWave US; `GUL82` = Pixel 10 Pro XL US). Run `pixel-uecaps-toolbox provision --help` to see all known codes, or call `provision` with any unknown code to trigger the full list. The model code determines which anchor-prime file is selected from the carrier's set (§3.2).
- **Donor carrier**: the carrier whose existing combos you will transplant. The donor and the target file must share the same SKU profile (same anchor prime), because `patch create` requires files of the same kind.

**Example**: target = VZW profile for Pixel 9 Pro XL mmWave (SKU 154921957), donor = ATT same profile. Both files (`VZW_132493905285110.binarypb`, `ATT_100936302644210.binarypb`) are confirmed in the corpus at that profile (§6.1).

#### Step 2 — Understand the existing profiles

Examine the donor and base before writing any patch:

```bash
# Inspect the base file
pixel-uecaps-toolbox inspect VZW_132493905285110.binarypb

# Inspect the donor
pixel-uecaps-toolbox inspect ATT_100936302644210.binarypb

# Compare them: see what ATT has that VZW does not, and vice versa
pixel-uecaps-toolbox compare VZW_132493905285110.binarypb ATT_100936302644210.binarypb
```

This surfaces the available combo space (toolbox README, `compare` reference). The combos advertised are a starting inventory — the toolbox transplants combos that exist in the donor; it does not synthesise new ones (see §6.3 Gap 1).

#### Step 3 — Author the change

**Route A — `patch create` / `filter` / `apply` (transplant selected combos)**

Use this route when you want to add a specific band or set of combos from one carrier's profile to another:

```bash
# 1. Build an A→B patch (ATT profile 154921957 as base, VZW same profile as target)
pixel-uecaps-toolbox patch create ATT_100936302644210.binarypb VZW_132493905285110.binarypb \
    -o att-to-vzw.patch.toml

# 2. (Optional) Filter to only the combos involving n77
pixel-uecaps-toolbox patch filter include n77 \
    --in att-to-vzw.patch.toml -o n77-only.patch.toml

# 3. Preview the filtered patch before applying
pixel-uecaps-toolbox patch show n77-only.patch.toml

# 4. Apply to the base file; keeps base identity (fingerprint, profile tag)
pixel-uecaps-toolbox patch apply ATT_100936302644210.binarypb \
    --in n77-only.patch.toml -o ATT_with_VZW_n77.binarypb
```

`patch apply` is best-effort by default: entries that don't fit the base are warned and skipped (use `--strict` to abort instead). Exit code `0` = clean, `1` = applied with skipped entries, `2` = error (toolbox README, `patch apply` reference).

`patch filter include`/`exclude` accepts band labels like `n77`, `B66`, or multiple at once. `--only` keeps only combos where *every* band is in the listed set (toolbox README, `patch filter` reference).

**Route B — `provision` (all-in-one per-Pixel-SKU module)**

Use this route when you want to target one specific Pixel model and combine multiple changes (LTE fallback, NR combos, PLMN legend) in a single step:

```bash
# Build a complete flashable module for Pixel 9 mmWave (G2YBB)
# targeting VZW, applying an NR patch and adding a PLMN to the legend
pixel-uecaps-toolbox provision G2YBB ~/Projects/reference/uecapconfig \
    --carrier VZW \
    --nr-patch vzw.nr.toml \
    --add-plmn 250-99 \
    -o vzw-p9-G2YBB.zip
```

`provision` selects the correct VZW file for the G2YBB anchor prime, applies the patch in memory, and packages the result. **Patch combos whose bands the Pixel model does not support (per the `pixel-bands` table) are silently skipped with a warning** (toolbox README, `provision` reference). This is the toolbox's per-model band ceiling enforcement; see §6.3 Gap 3 for the underlying hardware constraint.

The `--add-plmn` flag is refused if the PLMN is already mapped to any carrier in the legend (toolbox README). Use `mapping inject-plmn` directly for finer control:

```bash
pixel-uecaps-toolbox mapping inject-plmn VZW 250-99 \
    < ap_plmn_mapping.binarypb > new_mapping.binarypb
```

#### Step 4 — Package and deploy via `magisk`

After authoring the edited `.binarypb` file(s), package them for deployment:

```bash
# Package one edited carrier file
pixel-uecaps-toolbox magisk ATT_with_VZW_n77.binarypb -o uecaps-override.zip

# Or package multiple files at once (e.g., an edited carrier file + the edited legend)
pixel-uecaps-toolbox magisk ATT_with_VZW_n77.binarypb new_mapping.binarypb \
    -o uecaps-override.zip
```

The Magisk module overlays each file onto `/vendor/firmware/uecapconfig` (the default destination; override with `--dest`) using Magisk's systemless mount, leaving the stock partition untouched (toolbox README, `magisk` reference). Flash the `.zip` in the Magisk app (Modules → Install from storage) and reboot. Installing the module is root-only and at your own risk; a wrong capability set can break service (toolbox README disclaimer).

`provision` produces the same kind of Magisk module as `magisk` and can be flashed identically (toolbox README, `provision` reference).

#### Step 5 — Align cfgdb NR/EN-DC gating if needed

**This step is required whenever the band or mode change involves NR or EN-DC.** As established in §4.2 and §4.3 Step 9, cfgdb NV items gate the modem's behaviour while uecap defines its capability — both must simultaneously allow a band for the network to schedule it.

Specifically:

- The `endc_nr_ca_common` and `endc_nr_ca_common_manual` confseqs (present in all 439 carriers; §2.3) encode the baseline EN-DC / NR carrier-aggregation policy. If the serving carrier's instance of these confseqs disables EN-DC or NR, the modem will not attempt those modes even though it has advertised them via the patched uecap profile.
- The `wildcard-5g` module (215 carriers; §2.3) extends NR policy for a broad carrier set. `wildcard-5gsa` (1 carrier) and `rogers_5gsa` (1 carrier) handle NR-SA specifically.
- **If the target carrier's cfgdb does not permit the NR band mode you have added to the uecap profile**, Playbook A (§5.3) describes how to modify the cfgdb confseq bundle. Three routes are available: the framework route (§5.3.0, proven), the coarse cfgdb route (§5.3.1), and the surgical cfgdb NV-item route (§5.3.2, now unblocked — NV-item ids are crc32(name)).

For a purely LTE CA change (`lte_*.binarypb` patch), cfgdb NR/EN-DC gating is not relevant; `patch create lteA lteB` writes an `lte`-kind patch and `patch apply` reconstructs a new `lte_*.binarypb` (toolbox README, `patch` reference). The LTE fallback file selection is hardware-category-based, not SIM-based (§3.4).

#### Step 6 — Verify and rollback

**Verify**: after flashing and rebooting:

1. Confirm the modem reports the expected band via `adb shell dumpsys telephony.registry` or `adb shell cat /sys/class/net/rmnet_data0/queues/rx-0/…` (device-specific path; **[unverified]**).
2. Confirm the network connects on the new band (check LTE/NR band indicator in the Android signal overlay or a network-info app).
3. Confirm no regression on other carriers — a PLMN legend edit (`mapping inject-plmn`) applies globally across all carriers that share the legend file.

**Rollback**: remove the Magisk module in the Magisk app and reboot. The stock `/vendor/firmware/uecapconfig` partition is unmodified by the systemless overlay; removing the module restores the factory state. For `provision`-built modules, the stock files for all included profiles are restored on module removal.

---

### 6.3 Gap analysis

The toolbox is purpose-built for transplanting and filtering combos from *existing* carrier files. Three gaps exist at the boundaries of that capability.

#### Gap 1 — The patch model does not synthesize novel combos

`patch create` computes a set-difference between two existing files and records the result as a TOML patch of `delete` and `set` entries. `patch apply` replays those entries onto a base file. **The toolbox cannot add a combo that does not already appear in any file in the corpus.** Constructing a band combination that is entirely new — for example, adding a new NR-CA combo not present in any of the 1,389 carrier files (§3: 1,398 total − 1 legend − 8 `lte_*`) — would require directly constructing the `ComboGroup` → `Nested1` → `Nested2` → `ComboFeatures` protobuf structure (schema: `~/Projects/code/pixel-uecaps-toolbox/proto/ue_caps.proto`). No toolbox command assembles combo payloads from scratch; the patch TOML format encodes combos as opaque strings, not as structured capability tuples that can be programmatically constructed.

#### Gap 2 — Creating a brand-new carrier exceeds the patch model

Adding a carrier that does not exist in the dataset requires:

1. **A full set of `.binarypb` files** — up to 16 per carrier for the main tier (§3.3), each with the correct NUMBER (carrier-signature × SKU-anchor-prime; §3.2). The carrier signature must be chosen so it does not collide with any existing carrier's GCD.
2. **A PLMN legend entry** — `mapping inject-plmn` appends a PLMN to an existing carrier; creating an entirely new `Carrier` entry in the legend with a new `index` requires `mapping decode` → edit the TOML → `mapping encode`.
3. **No toolbox command generates files from scratch.** `provision` requires the carrier's files to already exist in the source directory — it selects, patches, and packages them, but does not create them (`provision` reference: "pulling files from a folder of capability files").

#### Gap 3 — Modem acceptance ceiling [inferred]

Even if a combo is present in the uecap profile and the cfgdb policy permits the mode, **the Shannon firmware will not sustain a connection on a band its RF hardware does not physically support**. This is the modem acceptance ceiling.

The evidence anchor is the `lte_*` selection mechanism (§3.4): the LTE fallback files are selected "modem-selected by hardware category 0x812 (Shannon g5400), not SIM/MCC" (toolbox README, `inspect lte_844857560.binarypb` output). The hardware category code is burned into the Shannon firmware, not derived from the carrier or SIM. This establishes that capability selection is keyed to physical hardware identity. **[inferred]** By the same principle, advertising a band in `UECapabilityInformation` that the device's RF front-end cannot tune does not create a working link — the network may attempt to schedule the band, but the modem's hardware cannot complete the transmission or reception. The `provision` command embodies this constraint: it warns and skips "patch combos whose bands the model doesn't support (per the `pixel-bands` table)" (toolbox README, `provision` reference), where `pixel-bands` encodes the RF capabilities per Google model code.

The practical implication: a uecap edit is bounded above by what the device's modem hardware already supports for other carriers. Transplanting combos that appear in another carrier's file for the same device SKU (same profile number, same family) is within the hardware ceiling by construction — that carrier's profile was authored for the same hardware. Adding bands from a different SKU profile (e.g., importing a mmWave combo from a profile 154921957 file into a sub-6 GHz only profile) may exceed the target device's hardware, and `provision` will silently drop those entries. **[inferred]**

---

## Part 7 — Open questions & future work

*What this part covers: claims still marked [inferred] or [unverified], gaps in the corpus analysis, what evidence would resolve each item, and the conditions under which tooling would be worth building.*

---

### 7.1 Manifest flag-field semantics (`flag1`, `f4`, `f8`)

**Status:** [unverified]

**What is known.** Every `ConfSeqRef` entry in a manifest proto carries three extra fields beyond `content_hash` (`field 2`):

| Field | Values observed | Corpus-wide count |
|-------|----------------|-------------------|
| `flag1` (field 1) | `1`, `2`, `3`, `4`, or absent | 1827 / 1827 / 790 / 6 / 2651 refs |
| `f4` (field 4) | `1` when present, absent otherwise | present 3892, absent 3209 of 7101 |
| `f8` (field 8) | always `4` | all 7101 refs |

The pairing of `flag1 = 1` with `.sim1` confseqs and `flag1 = 2` with `.sim2` confseqs is plausible [inferred] given the corpus counts, but the mapping is not directly verified. `f4 = 1` aligns in count (3892) with neither a simple per-slot split nor the count of any single confseq suffix; its role is unknown. `f8 = 4` on every single reference suggests a protocol constant or format sentinel, but the semantics are unknown.

**What would resolve it.** AOSP `CarrierConfigProvider` / `CarrierConfigLoader` source reading: a text-search for the field numbers (1, 4, 8) or a read of the manifest proto parsing code would confirm the frame semantics. Alternatively, building a manifest in which `flag1` values are systematically varied and observing which confseqs the telephony stack actually loads (via `adb logcat -s CarrierConfigLoader`) on a rooted device would give behavioural evidence.

---

### 7.2 NV-item id algorithm

**Status: RESOLVED [established]** (§2.5)

**What was resolved.** The NV-item `id` field (field 1 of `NvItem`) is `zlib.crc32(NV-item-name.encode('utf-8')) & 0xFFFFFFFF`. Cross-referencing the corpus against the g5400c Samsung Shannon modem NV table (60,153 named items) confirmed: (a) crc32 matches the table's own `crc32` column for all 60,153 rows (0 mismatches); (b) all 2,658 distinct ids in the 945 plain-protobuf confseqs resolve to a named NV item (100%); (c) all 354 distinct CLZ4 ids also resolve (100%); combined 2,831/2,831 (100%). Evidence: `analysis/cfgdb_nvitems.py` → `analysis/out/nvitems.txt`.

**Why the earlier cfgdb_keyhash.py sweep failed.** The sweep applied crc32 (and 7 other functions) to AOSP `CarrierConfigManager` key strings (`carrier_volte_available_bool`, etc.) — the wrong namespace. cfgdb confseqs contain Shannon modem NV items, not Android framework keys. The algorithm (crc32) was always correct; only the input namespace was wrong.

**Remaining NV-item question.** The g5400c NV table provides the name→id mapping but not value semantics. For string-typed NV items, values are char-code arrays (§2.4); for integer-typed items, the modem firmware's NV schema maps the id to its declared type. The NV schema itself is not in the static corpus.

---

### 7.3 `cfg.sha2` role and enforcement

**Status:** [unverified] (§2.7 Rule 3)

**What is known.** `cfg.sha2` = `b93504638520ebf0b406769ca6061d76e4bbd85c56c9ddedc94a25b4` (56 hex chars = 224 bits). Testing sha224, sha256, sha512_224, and sha256[:56]-char truncation of the current `cfg.db` yields no match (`integrity.txt`). The file already shipped in a state that would fail a standard-digest check, implying either: (a) the framework does not perform a byte-level integrity check of `cfg.db` using this value; (b) the value is computed at build time from a pre-WAL-checkpoint state of the database; or (c) the value is computed by a non-standard algorithm (e.g., hash of a canonical serialisation of the database content, not the raw file bytes).

**What would resolve it.** On a rooted Pixel device: (1) rename or corrupt `cfg.sha2` and observe whether the telephony framework logs an error or falls back to a built-in config; (2) strace the `CarrierConfigLoader` process during boot to see which files it opens and whether it reads `cfg.sha2`. If the file is not opened, enforcement is absent. If it is opened, the comparison algorithm can be identified by tracing the subsequent computation.

The practical implication for modification: until enforcement is confirmed, editing `cfg.db` without updating `cfg.sha2` is the only viable path (no reliable new value can be computed statically), and this is safe if enforcement is absent.

---

### 7.4 On-device cfgdb load path, modem provisioning path, and cfg.sha2 enforcement

**Status:** partially resolved — filesystem path [established]; Magisk overlay acceptance and cfg.sha2 enforcement remain [unverified] (§5.5)

**What is known.** The cfgdb package (directory containing `cfg.db`, `confseqs/`, `manifests/`, `cfg.sha2`, `build.info`) is shipped with the system image at **`/vendor/firmware/carrierconfig`** [established]. The telephony framework reads it at runtime via `CarrierConfigLoader`. The standard over-the-air update mechanism for cfgdb is GMS module delivery via Play Services. An alternative delivery path via a Magisk systemless overlay mounting at `/vendor/firmware/carrierconfig` is the confirmed target path, but whether the framework accepts the overlaid files is [unverified]. The on-device path by which the telephony framework provisions cfgdb NV items to the Shannon modem (Step 5b, §4.3) is [inferred] — the specific IPC mechanism, timing, and modem sideloading API are not known from static analysis alone.

**What would resolve it.**

- **Filesystem path**: confirmed as `/vendor/firmware/carrierconfig` [established].
- **Overlay acceptance**: mount a modified `cfg.db` via Magisk and observe whether `adb shell dumpsys carrier_config` reports the modified carrier config, or whether the framework re-reads from the original path.
- **Reload timing**: determine whether the telephony stack reloads cfgdb on a Magisk overlay mount without requiring a full reboot (it may cache the config at boot; a carrier-config refresh triggered by `adb shell am broadcast -a android.telephony.action.CARRIER_CONFIG_CHANGED` may or may not cause a re-read from disk).

---

### 7.5 Assembly precedence specifics

**Status:** [inferred] (§2.6)

**What is known.** The confman table `rowid` order defines the confseq assembly sequence [inferred]; higher-rowid entries override lower-rowid entries for the same NV-item id. Three further assembly questions are unresolved:

1. **Slot filtering order.** Whether the runtime first filters confseqs to the active SIM slot and then applies them in rowid order, or applies all confseqs and then discards slot-inappropriate keys, affects the final merged value when a `.common` confseq and a `.sim1` confseq both define the same key.
2. **`carrier_parent` interaction.** All 161 confmap parent-child pairs share the same confman table (`integrity.txt`: `share same confman table: 161`), so at the confseq-bundle level the parent contributes no distinct override layer. However, the `carrier_parent` table (187 rows; §2.2) covers a wider set and may drive a separate framework lookup phase (SIM-match fallback rather than confseq-merge).
3. **CLZ4 confseq role.** The 4× duplicated CLZ4 entry (hash `09600f3f9f68a2712a82d55133a935643a1a1dd5`) is present in every carrier's confman table (§2.3). CLZ4 decompression is now implemented (§7.6 / `analysis/cfgdb_nvitems.py`), so its NV items can be read; however, its specific role in the assembly order — whether its entries take precedence over or are superseded by later plain-protobuf confseqs — remains [inferred].

**What would resolve it.** AOSP `CarrierConfigLoader.java` or the modem sideloading code source reading. Specific search terms: `rowid`, `slot`, `confseq`, `confman`, `loadConfig`, `mergeConfig`. The runtime slot-filter ordering (filter-then-merge vs merge-then-filter) and the parent-inheritance timing also remain [inferred].

---

### 7.6 CLZ4 decompression

**Status: RESOLVED [established]** (§2.4, `analysis/cfgdb_nvitems.py` → `analysis/out/nvitems.txt`)

**What was resolved.** 104 of 1,055 confseq files begin with magic bytes `0x434c5a34` (`"CLZ4"`). These are a 16-byte proprietary header (`magic 4s | uncompressed_size I | compressed_size I | checksum I`, little-endian) followed by a standard LZ4 block payload. A pure-Python dependency-free decompressor was implemented in `analysis/cfgdb_nvitems.py` (`lz4_decompress_block` + `decompress_clz4`). Self-validation against all 104 CLZ4 files: 104/104 decompressed, parsed as `ConfSeqData` protobuf, and 100% of distinct ids found in the g5400c NV table. The decompressed payload is a plain `ConfSeqData` message (same schema as plain-protobuf confseqs). The 6 PEM-format confseqs (`-----BEGIN CERT…`) remain out of scope as distribution-convenience certificates.

**Editing CLZ4 confseqs**: decompress → edit NV items (id = crc32(name)) → recompress (LZ4 block, see `cfgdb_nvitems.py`) → re-prepend the 16-byte header → recompute `sha256(content)[:40]` (§2.7 Rule 1). The `cfgdb_nvitems.py` script provides the reference implementation for both decompress and compress paths.

---

### 7.7 Which modem NV items actually control IMS/VoLTE enablement

**Status:** [unverified] — research item (§5.3.2)

**What is known.** The g5400c NV table (60,153 entries) contains **1,024 items** matching any of the keywords `ims`, `volte`, `vonr`, `vowifi`, `wfc`, `sip` (case-insensitive substring match on NV-item name); **474** of these appear in corpus confseqs (plain + CLZ4). Evidence: `analysis/cfgdb_nvitems.py` § (6) → `analysis/out/nvitems.txt`. The 9 candidate items listed in §5.3.2 are all confirmed PRESENT in corpus confseqs; `NASU.VOLTE.CAPA`, `PSS.AIMS.VONR_CAPABILITY_INFO`, and `!SAEL3.VOLTE_CERTI` are confirmed absent (use modem defaults). Candidate items with the most likely control semantics (based on name) are `TCS_GV_SHANNON_VOLTE`, `UECAPA_REL15_IMS_VONR_FR1_SUPPORT`, `OMC.BASED.VONR.PLMN.ENABLE`, and the `PSS.AIMS.*` IMS stack parameter set. However, which specific NV items to set, to what values, and whether setting them is sufficient for IMS to register on a given carrier is **[unverified]** from static analysis.

**What would resolve it.** Two approaches:
1. **Corpus diff between IMS-capable and IMS-absent carriers**: compare the NV-item sets of a carrier known to have working IMS (e.g., `vzw`, `att`) against a carrier without IMS. The NV items present in the IMS-capable carrier but absent in the target are the surgical candidates. This is feasible with the current tooling (`cfgdb_nvitems.py` + Python set operations).
2. **On-device observation**: provision a modified confseq with a suspected NV item changed to an IMS-enabling value; observe whether the modem registers IMS via `adb shell dumpsys telephony.registry`. Live device validation remains out of scope for this document.

---

### 7.8 Modem acceptance ceiling

**Status:** [inferred] (§6.3 Gap 3)

**What is known.** The `provision` command in the toolbox warns and silently skips combos that reference bands not in the `pixel-bands` table for the target model code. The hardware-category code `0x812` identifying the Shannon g5400 modem governs LTE fallback file selection (§3.4). By analogy, the modem's RF front-end defines a hard ceiling on which NR bands are physically supportable; advertising a band above this ceiling in `UECapabilityInformation` is useless or harmful. The `pixel-bands` table in `uecaps_info.py` captures this per-model-code ceiling.

**What would resolve it.** Two complementary approaches:

1. **Cross-check `pixel-bands` against modem firmware**: extract the RF band support table from the Shannon firmware image (the `radio-mustang-g5400i-…img` referenced in `build.info`) and verify that every band in `pixel-bands` is present in the firmware's RF configuration.
2. **On-device band scan**: on a rooted Pixel, use `AT+CGBANDSET?` or equivalent Shannon AT command (if exposed) to query the modem's native band support and compare against the `pixel-bands` table.

---

### 7.9 What would justify building tooling

The analysis above is **static** — no maintained tooling is produced. The following conditions, if met, would each individually justify building a dedicated tool.

#### Condition 1 — Coarse Playbook A validated on device

If a Magisk overlay of a modified `cfg.db` is confirmed to (a) load cleanly with no integrity-check failure and (b) cause the telephony stack to use the modified confseq bundle for the target carrier, then a **cfgdb decode / edit / repack tool** is warranted. The tool would automate Steps 1–12 of the minimal valid-edit procedure (§2.7), resolving the shared-table cloning step and the manifest content-addressing chain. The round-trip proof (`ok=945 bad=0`; §5.1) already validates the serialisation kernel; only the orchestration layer (SQLite edits + manifest proto edits + hash re-computation) needs to be built.

#### Condition 2 — cfgdb NV-item decode/edit/repack tool (now feasible)

The key-hash algorithm is now identified (§7.2 RESOLVED: `id = crc32(NV-item-name)`), the NV table provides name lookup for 60,153 items, and CLZ4 decompression is implemented in pure Python (§7.6 RESOLVED). A **cfgdb NV-item decode/edit/repack tool** is therefore **feasible today** without any further research. The tool would: (a) decode any confseq to a human-readable NV-item list with symbolic names; (b) allow adding/removing/changing individual NV items by name (computing crc32(name) for the id automatically); (c) recompress CLZ4 confseqs; and (d) automate the sha256[:40] content-addressing and manifest/confman update chain. This is the single most valuable tool to build from the current corpus artifacts. The remaining prerequisite is on-device validation that a modified cfgdb package is accepted by the telephony framework (§7.4/Gate 3).

#### Condition 3 — From-scratch combo authoring needed

If the toolbox gap (§6.3 Gap 1 — "cannot add a combo not already in the corpus") becomes the limiting factor (e.g., a target band combination appears in no existing carrier file), a **combo-authoring extension** to the toolbox would be needed. This would require building a `ComboGroup` → `Nested1` → `Nested2` → `ComboFeatures` protobuf constructor from the `ue_caps.proto` schema and adding a `combo add` subcommand to the toolbox. The feature-set index management (`dl_feature_per_cc_list` / `ul_feature_per_cc_list`) would also need to be handled — new combos must reference existing feature-set entries or append new ones.

#### Condition 4 — New-carrier provisioning needed

If adding an entirely new carrier to `ap_plmn_mapping` and generating its full set of 16 `.binarypb` files (§6.3 Gap 2) becomes necessary, a **carrier-genesis command** is needed: it would compute a collision-free carrier signature (GCD-distinct from all existing carriers), generate one `.binarypb` file per SKU profile anchor prime, and register the carrier in the PLMN legend. This is the largest gap in the current toolbox — no existing command creates a full carrier entry from scratch.
