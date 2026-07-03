//! decompile — convert a cfgdb directory into an editable project.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};

use indexmap::IndexMap;

use crate::{
    cfgdb, clz4,
    confseq::ConfSeq,
    nvtable::NvTable,
    project::{CarrierFile, Lock, MatchRuleToml, Meta, ModuleToml, item_key},
};

/// Decompile a `cfgdb_dir` (containing `cfg.db`, `confseqs/`, etc.) into an
/// editable project rooted at `out`.
///
/// Directory layout produced:
/// ```text
/// out/
///   _meta.toml
///   carriers/<slug-or-id_N>.toml
///   originals/<40-hex-hash>
///   source/cfg.db  build.info  release-label  …
/// ```
pub fn decompile(cfgdb_dir: &Path, out: &Path, nv: &NvTable) -> crate::error::Result<()> {
    // 1. Read the database.
    let db = cfgdb::Cfgdb::read(cfgdb_dir)?;

    // 2. Create output directories.
    fs::create_dir_all(out.join("carriers"))?;
    fs::create_dir_all(out.join("source"))?;

    fs::create_dir_all(out.join("originals"))?;

    // `originals/` holds only the confseqs `compile` CANNOT rebuild from a carrier TOML:
    // CLZ4-compressed ones (lz4_flex won't reproduce the original compressor's bytes),
    // undecodable blobs (PEM certs), and unreferenced "orphans" (absent from every TOML).
    // Referenced plain confseqs are omitted — they re-encode byte-faithfully from the TOML.
    let mut locks: Vec<Lock> = Vec::new();
    let mut referenced: HashSet<String> = HashSet::new();
    let mut stored: HashSet<String> = HashSet::new();

    // 3. For each carrier, process its confseq hashes.
    for c in &db.carriers {
        let hashes = db.confman_confseqs(&c.confman)?;

        let carrier_slug = c
            .slug
            .clone()
            .unwrap_or_else(|| format!("id_{}", c.carrier_id));

        let mut seen: HashSet<String> = HashSet::new();
        let mut confseq_map: BTreeMap<String, ModuleToml> = BTreeMap::new();

        for hash in &hashes {
            referenced.insert(hash.clone());
            if !seen.insert(hash.clone()) {
                // Duplicate within this carrier — already processed.
                continue;
            }

            let bytes = fs::read(cfgdb_dir.join("confseqs").join(hash))?;
            let compressed = clz4::is_clz4(&bytes);

            // Attempt to decode as a protobuf ConfSeq.  Some entries are opaque
            // blobs (e.g. PEM certificates) with no editable TOML representation.
            match ConfSeq::decode(&bytes) {
                Ok(cs) => {
                    // Build items as an order-preserving map keyed by NV name (item_key).
                    // If two items collapse to one key (len shrinks) the confseq isn't
                    // faithfully representable, so fall through to the opaque path — in
                    // practice this is a repeated NV id (the wire lists the same id twice),
                    // not two distinct ids aliasing to one name.
                    let mut items: IndexMap<String, Vec<Vec<i64>>> = IndexMap::new();
                    for item in &cs.items {
                        items.insert(item_key(nv, item.id), item.vals.clone());
                    }
                    let representable = items.len() == cs.items.len();

                    if representable {
                        confseq_map.insert(
                            cs.name.clone(),
                            ModuleToml {
                                revision: cs.revision.clone(),
                                compressed,
                                items,
                            },
                        );
                        locks.push(Lock {
                            carrier: carrier_slug.clone(),
                            module: cs.name.clone(),
                            orig_hash: hash.clone(),
                        });
                        // CLZ4 can't be recompressed byte-faithfully → keep its verbatim
                        // bytes; a plain confseq re-encodes from the TOML, so don't store it.
                        if compressed && stored.insert(hash.clone()) {
                            fs::write(out.join("originals").join(hash), &bytes)?;
                        }
                    } else {
                        // Repeated NV-item id — the wire lists the same id twice, so the
                        // name-keyed map can't represent it. Store verbatim and treat as an
                        // opaque blob (no editable module).
                        locks.push(Lock {
                            carrier: carrier_slug.clone(),
                            module: format!("blob:{hash}"),
                            orig_hash: hash.clone(),
                        });
                        if stored.insert(hash.clone()) {
                            fs::write(out.join("originals").join(hash), &bytes)?;
                        }
                    }
                }
                Err(_) => {
                    // Opaque blob (e.g. PEM cert) — record a lock by hash and store it
                    // verbatim (nothing to re-encode it from).
                    locks.push(Lock {
                        carrier: carrier_slug.clone(),
                        module: format!("blob:{hash}"),
                        orig_hash: hash.clone(),
                    });
                    if stored.insert(hash.clone()) {
                        fs::write(out.join("originals").join(hash), &bytes)?;
                    }
                }
            }
        }

        // Build matching rules.
        let matching: Vec<MatchRuleToml> = c
            .matching
            .iter()
            .map(|r| MatchRuleToml {
                mccmnc: r.mccmnc.clone(),
                imsi_prefix_xpattern: r.imsi_prefix_xpattern.clone(),
                spn: r.spn.clone(),
                gid1: r.gid1.clone(),
                gid2: r.gid2.clone(),
                iccid_prefix: r.iccid_prefix.clone(),
            })
            .collect();

        let cf = CarrierFile {
            carrier_id: c.carrier_id,
            parent_id: c.parent_id,
            matching,
            confseq: confseq_map,
        };

        cf.write(&out.join("carriers").join(format!("{carrier_slug}.toml")))?;
    }

    // Store orphan confseqs (referenced by no carrier → absent from every TOML) verbatim,
    // so `compile` still reproduces them for a byte-faithful whole-directory round-trip.
    for entry in fs::read_dir(cfgdb_dir.join("confseqs"))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !referenced.contains(name.as_ref()) && stored.insert(name.to_string()) {
            fs::copy(entry.path(), out.join("originals").join(name.as_ref()))?;
        }
    }

    // 4. Self-contained stash: copy key files so `compile` needs only the project dir.
    for fname in &[
        "cfg.db",
        "build.info",
        "release-label",
        "confseqs_symbolic_link_mapping",
        "manifests_symbolic_link_mapping",
        "cfg.sha2",
    ] {
        let src = cfgdb_dir.join(fname);
        if src.exists() {
            fs::copy(&src, out.join("source").join(fname))?;
        }
    }

    // Stash the original `manifests/` dir verbatim.  `compile` copies an unedited
    // carrier's manifest byte-for-byte and rewrites an edited one in place (swapping
    // confseq refs, keeping the confman hash), so it needs the originals on hand.
    let manifests_src = cfgdb_dir.join("manifests");
    if manifests_src.is_dir() {
        copy_dir_files(&manifests_src, &out.join("source").join("manifests"))?;
    }

    // 5. Write _meta.toml.
    let release_label = read_trimmed(cfgdb_dir.join("release-label"));
    let build_info = read_trimmed(cfgdb_dir.join("build.info"));

    let meta = Meta {
        release_label,
        build_info,
        versions: db.versions.clone(),
        iin: None,
        regional_fallback: None,
        carrier_parent: None,
        nv_table: nv.label().to_string(),
        locks,
    };

    meta.write(&out.join("_meta.toml"))?;

    Ok(())
}

/// Read a text file and trim surrounding whitespace; return empty string on error.
fn read_trimmed(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Copy every regular file from `src` into `dst` (created if absent), verbatim.
fn copy_dir_files(src: &Path, dst: &Path) -> crate::error::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
