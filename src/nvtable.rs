//! crc32 <-> Shannon NV-item-name dictionary (baked-in, static).
//!
//! The `id -> name` table is a compile-time `phf::Map` generated once, out-of-band,
//! and committed as `src/nvtable_data.rs`. There is no runtime table loading: the g5400
//! map is the only source. `id(name)` is pure crc32, so no reverse table is stored. Ids
//! absent from the map surface to callers as `unknown_<crc32>`.

pub fn crc32_id(name: &str) -> u32 {
    crc32fast::hash(name.as_bytes())
}

/// Handle to the baked-in g5400 NV table. Zero-sized: all data is `static`.
pub struct NvTable;

impl NvTable {
    /// The bundled g5400 table. Infallible — the data is compiled in.
    pub fn bundled() -> NvTable {
        NvTable
    }

    /// Human-readable label identifying this NV table (e.g. `"g5400_nv"`).
    pub fn label(&self) -> &'static str {
        crate::nvtable_data::NV_LABEL
    }

    /// Resolve an NV-item id to its name, if the table knows it.
    pub fn name(&self, id: u32) -> Option<&'static str> {
        crate::nvtable_data::NV_NAMES.get(&id).copied()
    }

    /// The id for a name — always `crc32(name)`; no table lookup needed.
    pub fn id(&self, name: &str) -> u32 {
        crc32_id(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_ids() {
        assert_eq!(crc32_id("TCS_GV_OPT_CARRIER_TYPE"), 922_505_959);
        assert_eq!(crc32_id("gTCS_FCI_info"), 287_538_830);
        assert_eq!(crc32_id("NV.ITEM.GUARD"), 3_854_640_803);
    }

    #[test]
    fn bundled_resolves_names() {
        let t = NvTable::bundled();
        assert_eq!(t.name(922_505_959), Some("TCS_GV_OPT_CARRIER_TYPE"));
        assert_eq!(t.id("TCS_GV_OPT_CARRIER_TYPE"), 922_505_959);
        assert_eq!(t.name(1), None); // unknown id
    }

    /// Every entry must satisfy the table's defining law: id == crc32(name).
    /// Self-contained integrity check over all ~60k rows (no external file needed).
    #[test]
    fn nv_names_are_crc32_consistent() {
        for (id, name) in crate::nvtable_data::NV_NAMES.entries() {
            assert_eq!(crc32_id(name), *id, "id/name mismatch for {name:?}");
        }
    }
}
