//! confseq protobuf codec (hand-rolled varint) + content-hash.
use crate::{
    clz4,
    error::{Error, Result},
};
use sha2::{Digest, Sha256};

pub struct NvItem {
    pub id: u32,
    /// Values grouped exactly as the wire encodes them: one inner `Vec` per `Val`
    /// submessage (field 2), each holding that `Val`'s repeated field-3 values. An
    /// empty inner `Vec` is an empty `Val` (`12 00`). Preserving this grouping (rather
    /// than flattening) makes decode→encode byte-faithful.
    pub vals: Vec<Vec<i64>>,
}
pub struct ConfSeq {
    pub revision: String,
    pub name: String,
    pub items: Vec<NvItem>,
}

pub fn content_hash(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d[..20].iter().map(|b| format!("{b:02x}")).collect()
}

// ---- varint helpers ----
pub(crate) fn read_varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let (mut shift, mut out) = (0u32, 0u64);
    loop {
        let byte = *b
            .get(*i)
            .ok_or_else(|| Error::Confseq("varint eof".into()))?;
        *i += 1;
        out |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(out);
        }
        shift += 7;
    }
}
pub(crate) fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            return;
        }
    }
}
pub(crate) fn tag(field: u32, wire: u32) -> u64 {
    ((field as u64) << 3) | wire as u64
}
pub(crate) fn write_len_delim(out: &mut Vec<u8>, field: u32, payload: &[u8]) {
    write_varint(out, tag(field, 2));
    write_varint(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

// Generic single-message walker yielding (field, wire, &slice/value) — minimal for our schema.
pub(crate) enum Val<'a> {
    Var(u64),
    Len(&'a [u8]),
}
pub(crate) fn fields(b: &[u8]) -> Result<Vec<(u32, Val<'_>)>> {
    let (mut i, mut v) = (0, Vec::new());
    while i < b.len() {
        let t = read_varint(b, &mut i)?;
        let (field, wire) = ((t >> 3) as u32, (t & 7) as u32);
        match wire {
            0 => v.push((field, Val::Var(read_varint(b, &mut i)?))),
            2 => {
                let n = read_varint(b, &mut i)? as usize;
                let s = b
                    .get(i..i + n)
                    .ok_or_else(|| Error::Confseq("len eof".into()))?;
                i += n;
                v.push((field, Val::Len(s)));
            }
            5 => i += 4,
            1 => i += 8,
            w => return Err(Error::Confseq(format!("wire {w}"))),
        }
    }
    Ok(v)
}

impl ConfSeq {
    pub fn decode(buf: &[u8]) -> Result<ConfSeq> {
        let owned;
        let plain: &[u8] = if clz4::is_clz4(buf) {
            owned = clz4::decompress(buf)?;
            &owned
        } else {
            buf
        };
        let (mut revision, mut name, mut items) = (String::new(), String::new(), Vec::new());
        for (f, val) in fields(plain)? {
            match (f, val) {
                (1, Val::Len(s)) => revision = String::from_utf8_lossy(s).into_owned(),
                (2, Val::Len(s)) => name = String::from_utf8_lossy(s).into_owned(),
                (4, Val::Len(s)) => {
                    // one NvItem message
                    let (mut id, mut vals) = (0u32, Vec::new());
                    for (ff, vv) in fields(s)? {
                        match (ff, vv) {
                            (1, Val::Var(v)) => id = v as u32,
                            (2, Val::Len(item)) => {
                                // one Val message: its repeated field-3 values form one group
                                let mut group = Vec::new();
                                for (g, gv) in fields(item)? {
                                    if let (3, Val::Var(v)) = (g, gv) {
                                        group.push(v as i64);
                                    }
                                }
                                vals.push(group);
                            }
                            _ => {}
                        }
                    }
                    items.push(NvItem { id, vals });
                }
                _ => {}
            }
        }
        Ok(ConfSeq {
            revision,
            name,
            items,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_len_delim(&mut out, 1, self.revision.as_bytes());
        write_len_delim(&mut out, 2, self.name.as_bytes());
        for it in &self.items {
            let mut item = Vec::new();
            write_varint(&mut item, tag(1, 0));
            write_varint(&mut item, it.id as u64);
            for group in &it.vals {
                // one Val message per group (an empty group => an empty Val `12 00`)
                let mut val = Vec::new();
                for &v in group {
                    write_varint(&mut val, tag(3, 0));
                    write_varint(&mut val, v as u64);
                }
                write_len_delim(&mut item, 2, &val);
            }
            write_len_delim(&mut out, 4, &item);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encode_then_decode_roundtrips() {
        let cs = ConfSeq {
            revision: "v1.0".into(),
            name: "test.sim1".into(),
            items: vec![
                NvItem {
                    id: 922_505_959,
                    vals: vec![vec![4097]], // one Val, one value
                },
                NvItem {
                    id: 287_538_830,
                    vals: vec![vec![85, 83], vec![65, 53, 48]], // two distinct Val groups
                },
                NvItem {
                    id: 1,
                    vals: vec![vec![]], // a single EMPTY Val (the `12 00` case)
                },
            ],
        };
        let bytes = cs.encode();
        let back = ConfSeq::decode(&bytes).unwrap();
        assert_eq!(back.revision, "v1.0");
        assert_eq!(back.name, "test.sim1");
        assert_eq!(back.items.len(), 3);
        // grouping is preserved (not flattened) ...
        assert_eq!(back.items[1].vals, vec![vec![85, 83], vec![65, 53, 48]]);
        // ... and so is an empty Val.
        assert_eq!(back.items[2].vals, vec![Vec::<i64>::new()]);
        // re-encode is byte-stable for arbitrary grouping + empties.
        assert_eq!(back.encode(), bytes);
    }
    #[test]
    fn content_hash_is_40_hex() {
        let h = content_hash(b"abc");
        assert_eq!(h.len(), 40);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
