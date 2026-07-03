//! manifest protobuf — read-only decode + byte-faithful ref-hash rewrite.
//!
//! A manifest lists a carrier's confseqs (the same set as its `confman_<hash>` table).
//! We deliberately never re-encode a manifest: the wire format carries optional and
//! opaque fields (the `carrier_id` is sometimes absent — e.g. the wildcard carrier — and
//! refs carry flag fields 1,4,5,6,7,8 whose meaning is unverified). Re-encoding from a
//! typed model would silently drop anything un-modeled. Editing instead surgically
//! swaps the fixed-length 20-byte ref hash in the original bytes, preserving every other
//! byte. `decode` exists only to read the referenced hashes (e.g. for `check`'s
//! confman≡manifest invariant), not to round-trip.

use crate::{
    confseq::{Val, fields},
    error::{Error, Result},
};

/// A manifest reference to one confseq, by its 40-hex content hash.
pub struct Ref {
    pub hash: String,
}

pub struct Manifest {
    pub version: String,
    pub name: String,
    /// Carrier id, or 0 if the manifest omits field 3 (e.g. the wildcard carrier).
    pub carrier_id: u32,
    pub refs: Vec<Ref>,
}

impl Manifest {
    /// Parse a manifest, collecting each ref's confseq hash (field 2 of each field-5 ref).
    /// Opaque ref flag fields are skipped; only the hash is retained.
    pub fn decode(buf: &[u8]) -> Result<Manifest> {
        let (mut version, mut name, mut carrier_id, mut refs) =
            (String::new(), String::new(), 0u32, Vec::new());
        for (f, v) in fields(buf)? {
            match (f, v) {
                (1, Val::Len(s)) => version = String::from_utf8_lossy(s).into_owned(),
                (2, Val::Len(s)) => name = String::from_utf8_lossy(s).into_owned(),
                (3, Val::Var(x)) => carrier_id = x as u32,
                (5, Val::Len(r)) => {
                    let mut hash = String::new();
                    for (ff, vv) in fields(r)? {
                        if let (2, Val::Len(b)) = (ff, vv) {
                            hash = b.iter().map(|x| format!("{x:02x}")).collect();
                        }
                    }
                    refs.push(Ref { hash });
                }
                _ => {}
            }
        }
        Ok(Manifest {
            version,
            name,
            carrier_id,
            refs,
        })
    }
}

/// Convert a 40-char lowercase-hex string to its 20 raw bytes.
fn hash_to_bytes(h: &str) -> Result<[u8; 20]> {
    if h.len() != 40 {
        return Err(Error::Project(format!(
            "ref hash must be 40 hex chars, got {}: {h:?}",
            h.len()
        )));
    }
    let mut out = [0u8; 20];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)
            .map_err(|_| Error::Project(format!("non-hex ref hash: {h:?}")))?;
    }
    Ok(out)
}

/// Rewrite confseq ref hashes in a manifest's raw bytes, preserving every other byte.
///
/// Each ref hash is stored as a `12 14 <20 bytes>` field (field 2, length 20), so an
/// old→new substitution of equal length is byte-safe. Every occurrence is replaced
/// (a manifest may list one confseq up to 4×). Bytes outside the matched windows —
/// unmodeled fields, the optional `carrier_id`, flag ordering — are untouched, so an
/// edited manifest differs from the original only in the swapped hashes. A hash absent
/// from the manifest is simply not found (no-op); an empty `remap` returns the input
/// unchanged.
pub fn rewrite_ref_hashes(
    bytes: &[u8],
    remap: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();
    for (old, new) in remap {
        let (old_b, new_b) = (hash_to_bytes(old)?, hash_to_bytes(new)?);
        let mut i = 0;
        while i + 22 <= out.len() {
            if out[i] == 0x12 && out[i + 1] == 0x14 && out[i + 2..i + 22] == old_b {
                out[i + 2..i + 22].copy_from_slice(&new_b);
                i += 22;
            } else {
                i += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Build a field-5 ref entry: `12 14 <hash20>` then an opaque `40 04` (f8) flag.
    fn ref_entry(hash20: &[u8; 20]) -> Vec<u8> {
        let mut r = vec![0x12, 0x14];
        r.extend_from_slice(hash20);
        r.extend_from_slice(&[0x40, 0x04]); // an opaque flag we intentionally do not model
        let mut out = vec![0x2a, r.len() as u8];
        out.extend_from_slice(&r);
        out
    }

    /// Build a manifest: version "v0.1", name "x", `carrier_id` 5, then the given refs.
    fn manifest_bytes(hashes: &[[u8; 20]]) -> Vec<u8> {
        let mut b = vec![
            0x0a, 0x04, b'v', b'0', b'.', b'1', 0x12, 0x01, b'x', 0x18, 0x05,
        ];
        for h in hashes {
            b.extend(ref_entry(h));
        }
        b
    }

    fn hex(h: &[u8; 20]) -> String {
        h.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn decode_reads_version_name_carrier_and_hashes() {
        let (aa, bb) = ([0xaau8; 20], [0xbbu8; 20]);
        let m = Manifest::decode(&manifest_bytes(&[aa, bb])).unwrap();
        assert_eq!(m.version, "v0.1");
        assert_eq!(m.name, "x");
        assert_eq!(m.carrier_id, 5);
        assert_eq!(m.refs.len(), 2);
        assert_eq!(m.refs[0].hash, hex(&aa));
        assert_eq!(m.refs[1].hash, hex(&bb));
    }

    #[test]
    fn rewrite_swaps_only_the_targeted_hash_and_preserves_length() {
        let (aa, bb, cc) = ([0xaau8; 20], [0xbbu8; 20], [0xccu8; 20]);
        let bytes = manifest_bytes(&[aa, bb]);
        let mut remap = BTreeMap::new();
        remap.insert(hex(&aa), hex(&cc));
        let out = rewrite_ref_hashes(&bytes, &remap).unwrap();
        assert_eq!(out.len(), bytes.len(), "length must be preserved");
        let m = Manifest::decode(&out).unwrap();
        assert_eq!(m.refs[0].hash, hex(&cc), "aa should be swapped to cc");
        assert_eq!(m.refs[1].hash, hex(&bb), "bb must be untouched");
        assert_eq!(m.carrier_id, 5, "carrier_id preserved");
    }

    #[test]
    fn rewrite_replaces_all_duplicate_occurrences() {
        let (dup, new) = ([0x11u8; 20], [0x22u8; 20]);
        let bytes = manifest_bytes(&[dup, dup, dup, dup]); // the classic 4x duplicate
        let mut remap = BTreeMap::new();
        remap.insert(hex(&dup), hex(&new));
        let m = Manifest::decode(&rewrite_ref_hashes(&bytes, &remap).unwrap()).unwrap();
        assert_eq!(m.refs.len(), 4);
        assert!(m.refs.iter().all(|r| r.hash == hex(&new)), "all 4 swapped");
    }

    #[test]
    fn rewrite_with_empty_remap_is_identity() {
        let bytes = manifest_bytes(&[[0x33u8; 20]]);
        assert_eq!(rewrite_ref_hashes(&bytes, &BTreeMap::new()).unwrap(), bytes);
    }
}
