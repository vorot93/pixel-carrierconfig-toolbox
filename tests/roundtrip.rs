//! Corpus-gated integration tests.
//!
//! Set `CFGDB_CORPUS=<path-to-cfgdb-dir>` to run against a real corpus.
//! Without the variable the tests print "skip" and pass immediately.

use pixel_carrierconfig_toolbox as pct;

/// Create a fresh temporary directory for a single test run.
fn tempdir_like() -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Prefer the per-test-binary tmp dir Cargo provides at runtime.
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(format!("pct_decompile_{ts}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn decompile_writes_vzw() {
    let Some(dir) = std::env::var_os("CFGDB_CORPUS") else {
        eprintln!("skip: CFGDB_CORPUS not set");
        return;
    };
    let tmp = tempdir_like();
    let nv = pct::nvtable::NvTable::bundled();
    pct::project::decompile::decompile(dir.as_ref(), &tmp, &nv).unwrap();

    // us_vzw carrier file must exist and contain a known NV item name.
    let vzw = std::fs::read_to_string(tmp.join("carriers/us_vzw.toml")).unwrap();
    assert!(
        vzw.contains("TCS_GV_OPT_CARRIER_TYPE"),
        "us_vzw.toml missing TCS_GV_OPT_CARRIER_TYPE; snippet: {}",
        &vzw[..vzw.len().min(500)]
    );

    // Project-level metadata must exist.
    assert!(tmp.join("_meta.toml").exists(), "_meta.toml missing");

    // originals/ now holds ONLY what compile can't rebuild from a TOML (CLZ4 + blobs +
    // orphans) — strictly fewer than the full confseqs/ set, since referenced plain
    // confseqs are dropped (they re-encode byte-faithfully from the TOML at compile time).
    let corpus_confseqs = std::fs::read_dir(std::path::Path::new(&dir).join("confseqs"))
        .unwrap()
        .count();
    let originals = std::fs::read_dir(tmp.join("originals")).unwrap().count();
    assert!(
        originals > 0 && originals < corpus_confseqs,
        "originals/ should be a slim subset of confseqs/ (got {originals} of {corpus_confseqs})"
    );
    // A PEM-cert blob (undecodable, referenced) must still be kept verbatim.
    assert!(
        tmp.join("originals/745bd29be45667514b4000e9cdb70cdecad0f02c")
            .exists(),
        "PEM-cert confseq must be stored in originals/"
    );

    // The original manifests/ dir must be stashed so compile is self-contained.
    assert!(
        std::fs::read_dir(tmp.join("source/manifests"))
            .map(|d| d.count())
            .unwrap_or(0)
            > 0,
        "source/manifests/ is empty or missing"
    );
}

fn corpus() -> Option<std::path::PathBuf> {
    std::env::var_os("CFGDB_CORPUS").map(Into::into)
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) {
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        let d = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&d).unwrap();
            copy_tree(&p, &d);
        } else {
            std::fs::copy(&p, &d).unwrap();
        }
    }
}

fn assert_subdir_identical(a: &std::path::Path, b: &std::path::Path, label: &str) {
    let mut names: Vec<_> = std::fs::read_dir(a)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    for name in &names {
        let (fa, fb) = (a.join(name), b.join(name));
        assert!(fb.exists(), "{label}: {name:?} missing from output");
        assert_eq!(
            std::fs::read(&fa).unwrap(),
            std::fs::read(&fb).unwrap(),
            "{label}: {name:?} not byte-identical"
        );
    }
    let out_count = std::fs::read_dir(b).unwrap().count();
    assert_eq!(names.len(), out_count, "{label}: file count differs");
}

#[test]
fn roundtrip_is_byte_faithful() {
    let Some(corpus) = corpus() else {
        eprintln!("skip: CFGDB_CORPUS not set");
        return;
    };
    let (proj, out) = (tempdir_like(), tempdir_like());
    let nv = pct::nvtable::NvTable::bundled();
    pct::project::decompile::decompile(&corpus, &proj, &nv).unwrap();
    pct::project::compile::compile(&proj, &out, true).unwrap();
    // The whole device directory must come back byte-for-byte.
    assert_subdir_identical(&corpus.join("confseqs"), &out.join("confseqs"), "confseqs");
    assert_subdir_identical(
        &corpus.join("manifests"),
        &out.join("manifests"),
        "manifests",
    );
    for f in [
        "cfg.db",
        "cfg.sha2",
        "build.info",
        "release-label",
        "confseqs_symbolic_link_mapping",
        "manifests_symbolic_link_mapping",
    ] {
        let (a, b) = (corpus.join(f), out.join(f));
        if a.exists() {
            assert_eq!(
                std::fs::read(&a).unwrap(),
                std::fs::read(&b).unwrap(),
                "{f} not byte-identical"
            );
        }
    }
}

// _meta-lock helpers: read the (carrier, module) -> orig_hash locks recorded by decompile.
fn meta_locks(project: &std::path::Path) -> Vec<pct::project::Lock> {
    pct::project::Meta::read(&project.join("_meta.toml"))
        .unwrap()
        .locks
}
fn carrier_slugs(project: &std::path::Path) -> std::collections::BTreeSet<String> {
    meta_locks(project).into_iter().map(|l| l.carrier).collect()
}
fn carrier_hashes(project: &std::path::Path, slug: &str) -> std::collections::BTreeSet<String> {
    meta_locks(project)
        .into_iter()
        .filter(|l| l.carrier == slug)
        .map(|l| l.orig_hash)
        .collect()
}
fn orig_hash(project: &std::path::Path, slug: &str, module: &str) -> String {
    meta_locks(project)
        .into_iter()
        .find(|l| l.carrier == slug && l.module == module)
        .unwrap()
        .orig_hash
}
// Pick a carrier whose confman is referenced by exactly one carrier (unique), so an
// in-place confman/manifest update touches no other carrier — and which has an editable module.
fn pick_unique_confman_carrier(project: &std::path::Path) -> String {
    let db = pct::cfgdb::Cfgdb::read(&project.join("source")).unwrap();
    let mut share: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for c in &db.carriers {
        *share.entry(c.confman.clone()).or_default() += 1;
    }
    let mut slugs: Vec<String> = db
        .carriers
        .iter()
        .filter(|c| share[&c.confman] == 1)
        .filter_map(|c| c.slug.clone())
        .collect();
    slugs.sort();
    for slug in slugs {
        let cf =
            pct::project::CarrierFile::read(&project.join("carriers").join(format!("{slug}.toml")))
                .unwrap();
        if cf.confseq.values().any(|m| !m.items.is_empty()) {
            return slug;
        }
    }
    panic!("no unique-confman carrier with an editable module found");
}

#[test]
fn edit_isolates_to_the_edited_module() {
    let Some(corpus) = corpus() else {
        eprintln!("skip: CFGDB_CORPUS not set");
        return;
    };
    let nv = pct::nvtable::NvTable::bundled();
    let (orig, edit, out, redec) = (
        tempdir_like(),
        tempdir_like(),
        tempdir_like(),
        tempdir_like(),
    );
    pct::project::decompile::decompile(&corpus, &orig, &nv).unwrap();
    copy_tree(&orig, &edit);

    let slug = pick_unique_confman_carrier(&orig);
    let f = edit.join("carriers").join(format!("{slug}.toml"));
    let mut cf = pct::project::CarrierFile::read(&f).unwrap();
    let module = cf
        .confseq
        .iter()
        .find(|(_, m)| !m.items.is_empty())
        .map(|(k, _)| k.clone())
        .unwrap();
    let key = cf.confseq[&module].items.keys().next().unwrap().clone();
    // Append a value to the item's first Val group (changing its content); every item
    // has >=1 Val, but fall back to adding a group if somehow empty.
    let groups = cf
        .confseq
        .get_mut(&module)
        .unwrap()
        .items
        .get_mut(&key)
        .unwrap();
    match groups.first_mut() {
        Some(g) => g.push(123_456),
        None => groups.push(vec![123_456]),
    }
    cf.write(&f).unwrap();

    pct::project::compile::compile(&edit, &out, true).unwrap();
    pct::project::decompile::decompile(&out, &redec, &nv).unwrap();

    // (a) the edit round-trips through compile -> decompile
    let back =
        pct::project::CarrierFile::read(&redec.join("carriers").join(format!("{slug}.toml")))
            .unwrap();
    assert!(
        back.confseq[&module].items[&key]
            .iter()
            .any(|g| g.contains(&123_456)),
        "edit did not round-trip"
    );
    // (b) the edited module's confseq content-hash changed
    assert_ne!(
        orig_hash(&orig, &slug, &module),
        orig_hash(&redec, &slug, &module)
    );
    // (c) the confman is unique, so every OTHER carrier's confseq hashes are unchanged
    for s in carrier_slugs(&orig) {
        if s == slug {
            continue;
        }
        assert_eq!(
            carrier_hashes(&orig, &s),
            carrier_hashes(&redec, &s),
            "carrier {s} changed"
        );
    }
}

// Encoder-fidelity guards for the EDIT path. The round-trip test proves only that
// UNEDITED confseqs/manifests are byte-identical (they are copied verbatim). When a
// module is edited, compile re-encodes it from the decoded model — so the encoders
// must reproduce the original bytes with no loss.
//
// Manifests: byte-faithful via surgical hash-swap (their opaque ref fields carry real
// data). Confseqs: the grouped `vals: Vec<Vec<i64>>` model preserves Val grouping and
// empty Vals, so decode→encode is now BYTE-identical (not merely semantically equal).
// We compare against the plain (decompressed) protobuf; the CLZ4 compression layer is
// separate and not required to be byte-stable on re-compress.
#[test]
fn all_confseqs_reencode_byte_identical() {
    let Some(corpus) = corpus() else {
        eprintln!("skip: CFGDB_CORPUS not set");
        return;
    };
    let (mut checked, mut blobs) = (0u32, 0u32);
    for e in std::fs::read_dir(corpus.join("confseqs")).unwrap() {
        let p = e.unwrap().path();
        let raw = std::fs::read(&p).unwrap();
        let plain = if pct::clz4::is_clz4(&raw) {
            pct::clz4::decompress(&raw).unwrap()
        } else {
            raw.clone()
        };
        match pct::confseq::ConfSeq::decode(&raw) {
            Ok(cs) => {
                assert_eq!(
                    cs.encode(),
                    plain,
                    "confseq {:?} is not byte-identical through decode->encode",
                    p.file_name()
                );
                checked += 1;
            }
            // PEM certificate / opaque blob — never re-encoded (copied verbatim).
            Err(_) => blobs += 1,
        }
    }
    eprintln!("confseqs re-encoded byte-identical: {checked}; opaque blobs skipped: {blobs}");
    assert!(checked > 0, "no confseqs decoded");
}

#[test]
fn all_manifests_rewrite_is_byte_faithful() {
    let Some(corpus) = corpus() else {
        eprintln!("skip: CFGDB_CORPUS not set");
        return;
    };
    let mut checked = 0u32;
    for e in std::fs::read_dir(corpus.join("manifests")).unwrap() {
        let p = e.unwrap().path();
        let raw = std::fs::read(&p).unwrap();
        // A no-op rewrite must return the original bytes exactly (proves no corruption).
        assert_eq!(
            pct::manifest::rewrite_ref_hashes(&raw, &std::collections::BTreeMap::new()).unwrap(),
            raw,
            "manifest {:?} not preserved by no-op rewrite",
            p.file_name()
        );
        // A real swap must change only the targeted hash, preserving length & all else.
        let before = pct::manifest::Manifest::decode(&raw).unwrap();
        if let Some(old) = before.refs.first().map(|r| r.hash.clone()) {
            let new = "f".repeat(40);
            let mut remap = std::collections::BTreeMap::new();
            remap.insert(old.clone(), new.clone());
            let out = pct::manifest::rewrite_ref_hashes(&raw, &remap).unwrap();
            assert_eq!(
                out.len(),
                raw.len(),
                "manifest {:?} length changed",
                p.file_name()
            );
            let after = pct::manifest::Manifest::decode(&out).unwrap();
            assert_eq!(before.carrier_id, after.carrier_id);
            assert_eq!(before.name, after.name);
            assert_eq!(before.refs.len(), after.refs.len());
            for (b, a) in before.refs.iter().zip(after.refs.iter()) {
                let expect = if b.hash == old { &new } else { &b.hash };
                assert_eq!(
                    &a.hash,
                    expect,
                    "manifest {:?} a ref changed unexpectedly",
                    p.file_name()
                );
            }
        }
        checked += 1;
    }
    eprintln!("manifests rewrite byte-faithful: {checked}");
    assert!(checked > 0, "no manifests found");
}
