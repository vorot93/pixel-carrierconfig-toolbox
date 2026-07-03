//! CLZ4 container: 16-byte header + an LZ4 block.
use crate::error::{Error, Result};

const MAGIC: &[u8; 4] = b"CLZ4";

pub fn is_clz4(buf: &[u8]) -> bool {
    buf.len() >= 16 && buf.starts_with(MAGIC)
}

pub fn decompress(buf: &[u8]) -> Result<Vec<u8>> {
    if !is_clz4(buf) {
        return Err(Error::Clz4("bad magic/too short".into()));
    }
    let uncompressed = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let compressed = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
    let payload = buf
        .get(16..16 + compressed)
        .ok_or_else(|| Error::Clz4("truncated payload".into()))?;
    lz4_flex::block::decompress(payload, uncompressed).map_err(|e| Error::Clz4(e.to_string()))
}

pub fn compress(plain: &[u8]) -> Vec<u8> {
    let block = lz4_flex::block::compress(plain);
    let mut out = Vec::with_capacity(16 + block.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    out.extend_from_slice(&(block.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // checksum: not verified by firmware
    out.extend_from_slice(&block);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_identity() {
        let plain = b"hello hello hello world world NV.ITEM payload payload".repeat(4);
        let c = compress(&plain);
        assert!(is_clz4(&c));
        assert_eq!(decompress(&c).unwrap(), plain);
    }

    #[test]
    fn header_fields() {
        let plain = vec![7u8; 1000];
        let c = compress(&plain);
        assert_eq!(&c[0..4], b"CLZ4");
        assert_eq!(u32::from_le_bytes(c[4..8].try_into().unwrap()), 1000); // uncompressed size
    }
}
