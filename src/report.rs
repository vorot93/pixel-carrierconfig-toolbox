//! report — inspect / check / self-test commands.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{
    cfgdb::Cfgdb,
    clz4,
    confseq::{ConfSeq, NvItem, content_hash},
    manifest::Manifest,
    nvtable::crc32_id,
    project::{CarrierFile, Meta, key_to_id},
};

/// Run built-in codec sanity checks.  Bails on the first failure; prints "self-test: ok" and
/// returns `Ok(())` on success.  Uses in-code fixtures only — no corpus, no filesystem.
pub fn self_test() -> anyhow::Result<()> {
    // 1. crc32_id known value.
    let id = crc32_id("TCS_GV_OPT_CARRIER_TYPE");
    if id != 922_505_959 {
        anyhow::bail!("crc32_id(\"TCS_GV_OPT_CARRIER_TYPE\") = {id}, want 922505959");
    }

    // 2. clz4 round-trip + is_clz4 discrimination.
    let data: Vec<u8> = (0u8..=255u8).collect(); // 256-byte pattern
    let compressed = clz4::compress(&data);
    if !clz4::is_clz4(&compressed) {
        anyhow::bail!("clz4::is_clz4 returned false for a compressed blob");
    }
    if clz4::is_clz4(&data) {
        anyhow::bail!("clz4::is_clz4 returned true for plaintext data");
    }
    let decompressed = clz4::decompress(&compressed)?;
    if decompressed != data {
        anyhow::bail!("clz4 round-trip: decompressed != original");
    }

    // 3. content_hash: 40 lowercase hex chars.
    let h = content_hash(b"pixel-carrierconfig-toolbox self-test");
    if h.len() != 40 {
        anyhow::bail!("content_hash length = {}, want 40", h.len());
    }
    if !h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        anyhow::bail!("content_hash is not all lowercase hex: {h:?}");
    }

    // 4. ConfSeq encode → decode round-trip (byte-identical re-encode).
    let cs = ConfSeq {
        revision: "v1.0".to_string(),
        name: "test.sim1".to_string(),
        items: vec![
            NvItem {
                id: 922_505_959,
                vals: vec![vec![4097]],
            },
            NvItem {
                id: 287_538_830,
                vals: vec![vec![85, 83], vec![65, 53, 48]], // multiple Val groups
            },
            NvItem {
                id: 1,
                vals: vec![vec![]], // an empty Val
            },
        ],
    };
    let encoded = cs.encode();
    let decoded = ConfSeq::decode(&encoded)?;
    if decoded.revision != cs.revision || decoded.name != cs.name {
        anyhow::bail!("ConfSeq decode: revision/name mismatch");
    }
    if decoded.items.len() != cs.items.len() {
        anyhow::bail!(
            "ConfSeq decode: {} items, want {}",
            decoded.items.len(),
            cs.items.len()
        );
    }
    let re_encoded = decoded.encode();
    if re_encoded != encoded {
        anyhow::bail!("ConfSeq encode→decode→encode is not byte-identical");
    }

    println!("self-test: ok");
    Ok(())
}

/// Print one carrier's NV items.
///
/// Loads `carriers/<slug>.toml`; errors clearly if absent.  With `full = true`, also prints each
/// module's `orig_hash` (from `_meta.toml` locks) and the crc32 id for every item.
pub fn inspect(project_dir: &Path, slug: &str, full: bool) -> anyhow::Result<()> {
    let carrier_path = project_dir.join("carriers").join(format!("{slug}.toml"));
    if !carrier_path.exists() {
        anyhow::bail!("carrier not found: {}", carrier_path.display());
    }
    let cf = CarrierFile::read(&carrier_path)?;

    // Build module → orig_hash map from _meta.toml locks (only needed for --full).
    let meta_path = project_dir.join("_meta.toml");
    let lock_map: HashMap<String, String> = if full && meta_path.exists() {
        Meta::read(&meta_path)?
            .locks
            .into_iter()
            .filter(|l| l.carrier == slug)
            .map(|l| (l.module, l.orig_hash))
            .collect()
    } else {
        HashMap::new()
    };
    if full && !meta_path.exists() {
        eprintln!("note: _meta.toml not found — orig_hash is unavailable for --full");
    }

    // Header line.
    let parent_str = cf
        .parent_id
        .map(|p| format!(" parent {p}"))
        .unwrap_or_default();
    println!("carrier {slug} (id {}){parent_str}", cf.carrier_id);

    // Modules are a BTreeMap — already sorted.
    for (module_name, module) in &cf.confseq {
        if full {
            let orig = lock_map.get(module_name).map_or("unknown", String::as_str);
            println!("  [{module_name}]  (orig_hash {orig})");
        } else {
            println!("  [{module_name}]");
        }

        // Items are a BTreeMap — already sorted.
        for (key, groups) in &module.items {
            // Render the Val grouping faithfully, e.g. `[[1, 2], [3]]` (matches the TOML).
            let vals: String = groups
                .iter()
                .map(|g| {
                    let inner = g
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("[{inner}]")
                })
                .collect::<Vec<_>>()
                .join(", ");
            if full {
                let id = key_to_id(key);
                println!("    {key} (id {id}) = [{vals}]");
            } else {
                println!("    {key} = [{vals}]");
            }
        }
    }

    Ok(())
}

/// Validate a project's integrity.
///
/// Checks:
/// 1. Every lock is reproducible: either its `orig_hash` is stored in `originals/`
///    (CLZ4 / blob / orphan), or its module is re-encodable from the carrier TOML
///    (a referenced plain confseq). A lock that is neither → `Err`.
/// 2. For each distinct confman in `source/cfg.db`, the multiset of confseq hashes from the
///    `confman_<hash>` table equals the multiset of hashes from `source/manifests/<hash>`.
///    A mismatch → `Err`.
/// 3. Counts NV items whose key is `unknown_<id>` (id not in the bundled table) and reports the
///    total (informational; not an error).
pub fn check(project_dir: &Path) -> anyhow::Result<()> {
    // 1. Load project metadata.
    let meta = Meta::read(&project_dir.join("_meta.toml"))?;

    // 2. Every lock must be reproducible — stored verbatim OR re-encodable from its TOML.
    let mut cf_cache: HashMap<String, Option<CarrierFile>> = HashMap::new();
    for lock in &meta.locks {
        if project_dir.join("originals").join(&lock.orig_hash).exists() {
            continue; // stored verbatim (CLZ4 / blob / orphan)
        }
        // Not stored → must be a referenced plain module present in the carrier TOML.
        let cf = cf_cache.entry(lock.carrier.clone()).or_insert_with(|| {
            CarrierFile::read(
                &project_dir
                    .join("carriers")
                    .join(format!("{}.toml", lock.carrier)),
            )
            .ok()
        });
        let reencodable = cf
            .as_ref()
            .is_some_and(|c| c.confseq.contains_key(&lock.module));
        if !reencodable {
            anyhow::bail!(
                "dangling lock: carrier={} module={} orig_hash={} — neither stored in originals/ nor re-encodable from the carrier TOML",
                lock.carrier,
                lock.module,
                lock.orig_hash
            );
        }
    }

    // 3. confman ≡ manifest set-identity (only when source/cfg.db is present).
    let source_dir = project_dir.join("source");
    if source_dir.join("cfg.db").exists() {
        let db = Cfgdb::read(&source_dir)?;
        let mut seen: HashSet<String> = HashSet::new();
        for carrier in &db.carriers {
            if !seen.insert(carrier.confman.clone()) {
                continue;
            }
            let confman = &carrier.confman;
            let manifest_path = source_dir.join("manifests").join(confman);
            if !manifest_path.exists() {
                eprintln!(
                    "warning: confman {confman} has no stashed manifest at source/manifests/ — skipping its set-identity check"
                );
                continue;
            }

            let mut db_hashes = db.confman_confseqs(confman)?;
            db_hashes.sort_unstable();

            let raw = std::fs::read(&manifest_path)?;
            let manifest = Manifest::decode(&raw)?;
            let mut manifest_hashes: Vec<String> =
                manifest.refs.iter().map(|r| r.hash.clone()).collect();
            manifest_hashes.sort_unstable();

            if db_hashes != manifest_hashes {
                let db_only = db_hashes.iter().find(|h| !manifest_hashes.contains(h));
                let mf_only = manifest_hashes.iter().find(|h| !db_hashes.contains(h));
                anyhow::bail!(
                    "confman {confman}: set mismatch — db has {} hash(es), manifest has {} \
                     (db-only e.g. {:?}, manifest-only e.g. {:?})",
                    db_hashes.len(),
                    manifest_hashes.len(),
                    db_only,
                    mf_only
                );
            }
        }
    }

    // 4. Count unknown-id items (informational).
    let carriers_dir = project_dir.join("carriers");
    let mut unknown_count = 0usize;
    if carriers_dir.is_dir() {
        for entry in std::fs::read_dir(&carriers_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("toml") {
                let cf = CarrierFile::read(&entry.path())?;
                for module in cf.confseq.values() {
                    for key in module.items.keys() {
                        if key.starts_with("unknown_") {
                            unknown_count += 1;
                        }
                    }
                }
            }
        }
    }

    println!("check: ok ({unknown_count} unknown-id items)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{CarrierFile, Lock, Meta, ModuleToml};
    use std::collections::BTreeMap;

    /// Create a scratch directory scoped to this test binary run.
    fn tempdir_like(tag: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = std::env::var("CARGO_TARGET_TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let dir = base.join(format!("pct_report_{tag}_{ts}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn self_test_passes() {
        super::self_test().unwrap();
    }

    #[test]
    fn inspect_returns_ok_on_minimal_project() {
        let project_dir = tempdir_like("inspect");
        std::fs::create_dir_all(project_dir.join("carriers")).unwrap();

        let mut items = indexmap::IndexMap::new();
        items.insert("TCS_GV_OPT_CARRIER_TYPE".to_string(), vec![vec![4097i64]]);
        let mut confseq = BTreeMap::new();
        confseq.insert(
            "core.sim1".to_string(),
            ModuleToml {
                revision: "v1.0".to_string(),
                compressed: false,
                items,
            },
        );
        let cf = CarrierFile {
            carrier_id: 1839,
            parent_id: None,
            matching: vec![],
            confseq,
        };
        cf.write(&project_dir.join("carriers/x.toml")).unwrap();

        // Minimal _meta.toml with one lock (inspect --full reads it for orig_hash).
        let meta = Meta {
            release_label: "test".to_string(),
            build_info: "test".to_string(),
            versions: vec![],
            iin: None,
            regional_fallback: None,
            carrier_parent: None,
            nv_table: "g5400_nv".to_string(),
            locks: vec![Lock {
                carrier: "x".to_string(),
                module: "core.sim1".to_string(),
                orig_hash: "a".repeat(40),
            }],
        };
        meta.write(&project_dir.join("_meta.toml")).unwrap();

        assert!(
            inspect(&project_dir, "x", false).is_ok(),
            "inspect (brief) should return Ok"
        );
        assert!(
            inspect(&project_dir, "x", true).is_ok(),
            "inspect --full should return Ok"
        );
    }

    #[test]
    fn inspect_errs_on_missing_carrier() {
        let project_dir = tempdir_like("inspect_miss");
        std::fs::create_dir_all(project_dir.join("carriers")).unwrap();
        assert!(
            inspect(&project_dir, "nonexistent", false).is_err(),
            "inspect on absent carrier should return Err"
        );
    }

    #[test]
    fn check_dangling_lock_returns_err() {
        let project_dir = tempdir_like("check_dangle");
        std::fs::create_dir_all(project_dir.join("originals")).unwrap();
        std::fs::create_dir_all(project_dir.join("carriers")).unwrap();

        // A lock whose orig_hash file does NOT exist in originals/.
        let meta = Meta {
            release_label: "test".to_string(),
            build_info: "test".to_string(),
            versions: vec![],
            iin: None,
            regional_fallback: None,
            carrier_parent: None,
            nv_table: "g5400_nv".to_string(),
            locks: vec![Lock {
                carrier: "x".to_string(),
                module: "core.sim1".to_string(),
                orig_hash: "deadbeef".repeat(5), // 40-char hex; file is absent
            }],
        };
        meta.write(&project_dir.join("_meta.toml")).unwrap();

        let result = check(&project_dir);
        assert!(result.is_err(), "expected Err for dangling lock, got Ok");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("dangling lock"),
            "error should mention 'dangling lock'; got: {msg}"
        );
    }

    #[test]
    fn check_ok_on_corpus() {
        let Some(corpus) = std::env::var_os("CFGDB_CORPUS").map(std::path::PathBuf::from) else {
            eprintln!("skip: set CFGDB_CORPUS");
            return;
        };
        let project_dir = tempdir_like("check_corpus");
        let nv = crate::nvtable::NvTable::bundled();
        crate::project::decompile::decompile(&corpus, &project_dir, &nv).unwrap();
        check(&project_dir).unwrap();
    }
}
