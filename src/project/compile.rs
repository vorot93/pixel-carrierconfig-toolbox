//! compile — recompile an editable project back into a cfgdb directory.
//!
//! Strategy (P1): reproduce the device directory by copying the verbatim
//! snapshot stashed by `decompile`, then re-encode **only** the confseqs whose
//! decoded contents actually changed.  Unchanged modules are never re-encoded —
//! they ride through byte-for-byte from `originals/` — so a decompile→compile
//! round-trip with no edits is byte-faithful regardless of encoder byte-stability.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::Path,
};

use rusqlite::Connection;

use crate::{
    cfgdb::Cfgdb,
    clz4,
    confseq::{ConfSeq, NvItem, content_hash},
    error::{Error, Result},
    manifest::rewrite_ref_hashes,
    project::{CarrierFile, Meta, key_to_id},
};

/// Recompile the editable project at `project_dir` back into a cfgdb directory at `out`.
///
/// `keep_sha2` is retained for API stability; `cfg.sha2` is always copied verbatim
/// (the device algorithm is unverified, so recompute is unsupported in P1).
pub fn compile(project_dir: &Path, out: &Path, keep_sha2: bool) -> Result<()> {
    // --- 1. Load -----------------------------------------------------------
    let source = project_dir.join("source");
    let db = Cfgdb::read(&source)?;
    let meta = Meta::read(&project_dir.join("_meta.toml"))?;

    // Read every carrier TOML, keyed by slug (= file stem).
    let mut carriers: BTreeMap<String, CarrierFile> = BTreeMap::new();
    for entry in fs::read_dir(project_dir.join("carriers"))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Project(format!("bad carrier filename: {path:?}")))?
            .to_string();
        carriers.insert(slug, CarrierFile::read(&path)?);
    }

    // Build (carrier_slug, module_name) -> orig_hash from the recorded locks.
    // Skip `blob:` modules: they have no TOML entry and are never edited in P1.
    let mut locks: HashMap<(String, String), String> = HashMap::new();
    for l in &meta.locks {
        if l.module.starts_with("blob:") {
            continue;
        }
        locks.insert((l.carrier.clone(), l.module.clone()), l.orig_hash.clone());
    }

    // --- 2. Reproduce the static files verbatim ----------------------------
    fs::create_dir_all(out)?;
    let out_confseqs = out.join("confseqs");
    let out_manifests = out.join("manifests");
    // `originals/` holds only the confseqs that can't be rebuilt from a TOML (CLZ4,
    // blobs, orphans); copy them verbatim. Referenced plain confseqs are written by
    // step 3 (re-encoded from the carrier TOMLs).
    copy_dir_files(&project_dir.join("originals"), &out_confseqs)?;
    copy_dir_files(&source.join("manifests"), &out_manifests)?;

    fs::copy(source.join("cfg.db"), out.join("cfg.db"))?;
    for fname in [
        "build.info",
        "release-label",
        "confseqs_symbolic_link_mapping",
        "manifests_symbolic_link_mapping",
    ] {
        let src = source.join(fname);
        if src.exists() {
            fs::copy(&src, out.join(fname))?;
        }
    }
    // cfg.sha2: always copy verbatim — the on-device value is NOT sha224(cfg.db)
    // and the real algorithm is unverified, so recompute is unsupported in P1.
    let sha2 = source.join("cfg.sha2");
    if sha2.exists() {
        fs::copy(&sha2, out.join("cfg.sha2"))?;
        if !keep_sha2 {
            eprintln!(
                "warning: cfg.sha2 recompute is unsupported (device algorithm unverified); \
                 copied the original cfg.sha2 verbatim"
            );
        }
    }

    // --- 3. Reproduce/edit each carrier's confseqs → remap (confman, orig_hash) → new ---
    let originals = project_dir.join("originals");
    let mut remap: HashMap<(String, String), String> = HashMap::new();

    for carrier in &db.carriers {
        let slug = carrier
            .slug
            .clone()
            .unwrap_or_else(|| format!("id_{}", carrier.carrier_id));
        let Some(cf) = carriers.get(&slug) else {
            continue;
        };

        for (module_name, m) in &cf.confseq {
            let orig_hash = locks
                .get(&(slug.clone(), module_name.clone()))
                .ok_or_else(|| {
                    Error::Project(format!(
                        "no lock recorded for carrier {slug} module {module_name}; re-run decompile"
                    ))
                })?;

            // Rebuild the ConfSeq from the (possibly edited) TOML.
            let rebuilt = ConfSeq {
                revision: m.revision.clone(),
                name: module_name.clone(),
                items: m
                    .items
                    .iter()
                    .map(|(k, vals)| NvItem {
                        id: key_to_id(k),
                        vals: vals.clone(),
                    })
                    .collect(),
            };

            // Two reproduction paths, decided by whether decompile stored the original:
            //   stored (CLZ4 — recompression isn't byte-faithful): reuse the verbatim
            //     bytes (already copied in step 2) unless the decoded content changed.
            //   not stored (referenced plain): re-encode from the TOML, which is
            //     byte-faithful, and write it (step 2 did not copy it).
            let orig_path = originals.join(orig_hash);
            let new_hash: String;
            if orig_path.exists() {
                let orig_cs = ConfSeq::decode(&fs::read(&orig_path)?)?;
                if orig_cs.revision == rebuilt.revision
                    && item_map(&orig_cs.items) == item_map(&rebuilt.items)
                {
                    continue; // unchanged — original already in out/confseqs/ from step 2
                }
                let bytes = if m.compressed {
                    clz4::compress(&rebuilt.encode())
                } else {
                    rebuilt.encode()
                };
                new_hash = content_hash(&bytes);
                fs::write(out_confseqs.join(&new_hash), &bytes)?;
            } else {
                // Referenced plain confseq: re-encoded bytes equal the original when
                // unedited (verified byte-faithful), so writing it reproduces it exactly.
                let bytes = rebuilt.encode();
                new_hash = content_hash(&bytes);
                fs::write(out_confseqs.join(&new_hash), &bytes)?;
                if &new_hash == orig_hash {
                    continue; // unchanged: byte-identical, nothing to remap
                }
            }

            // Changed → remap this confman's reference; bail on divergent shared edits.
            let key = (carrier.confman.clone(), orig_hash.clone());
            if let Some(prev) = remap.insert(key, new_hash.clone())
                && prev != new_hash
            {
                return Err(Error::Project(format!(
                    "carriers sharing confman {} have divergent edits to {}; \
                     editing carriers that share a confman needs P2 confman-forking",
                    carrier.confman, orig_hash
                )));
            }
        }
    }

    // --- 4. Apply the remap to manifests + cfg.db (changed confmans only) --
    let changed_confmans: BTreeSet<String> = remap.keys().map(|(c, _)| c.clone()).collect();
    if changed_confmans.is_empty() {
        // No edits: leave the copied cfg.db / manifests byte-identical.
        return Ok(());
    }

    let conn = Connection::open(out.join("cfg.db"))?;
    for confman in &changed_confmans {
        // Validate the confman up front: it is interpolated into both filesystem paths
        // (manifest read/write below) and a SQL table name, so guard it as 40 lowercase
        // hex chars before any such use to prevent path traversal / SQL injection from a
        // crafted cfg.db.
        check_confman_hex(confman)?;

        // Manifest: surgically swap this confman's edited ref hashes in the original
        // bytes (byte-faithful — never re-encode), keeping the same filename.
        let sub: BTreeMap<String, String> = remap
            .iter()
            .filter(|((c, _), _)| c == confman)
            .map(|((_, old), new)| (old.clone(), new.clone()))
            .collect();
        let original = fs::read(source.join("manifests").join(confman))?;
        fs::write(
            out_manifests.join(confman),
            rewrite_ref_hashes(&original, &sub)?,
        )?;

        // cfg.db: rewrite the confseq column of this confman's table.
        let sql = format!("UPDATE confman_{confman} SET confseq = ?1 WHERE confseq = ?2");
        for ((c, orig), new) in &remap {
            if c == confman {
                conn.execute(&sql, rusqlite::params![new, orig])?;
            }
        }
    }

    Ok(())
}

/// Map a confseq's items to `{id -> Val groups}` for an order-insensitive comparison.
/// Comparing the full grouped structure means a module counts as "unchanged" only when
/// its `Val` grouping (and empty `Val`s) also match — consistent with byte-faithful re-encode.
fn item_map(items: &[NvItem]) -> BTreeMap<u32, Vec<Vec<i64>>> {
    items.iter().map(|it| (it.id, it.vals.clone())).collect()
}

/// Guard a confman against SQL injection: must be exactly 40 lowercase hex chars
/// (the same rule `cfgdb::confman_confseqs` applies before interpolating a table name).
fn check_confman_hex(confman: &str) -> Result<()> {
    if confman.len() != 40
        || !confman
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(Error::Project(format!(
            "confman must be exactly 40 lowercase hex chars, got: {confman:?}"
        )));
    }
    Ok(())
}

/// Copy every regular file from `src` into `dst` (created if absent), verbatim.
fn copy_dir_files(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}
