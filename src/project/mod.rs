//! project — editable TOML data model for carrier configuration.
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::error::{Error, Result};
use crate::nvtable::NvTable;

pub mod compile;
pub mod decompile;

/// A single SIM-identity matching rule (TOML-serialisable mirror of `cfgdb::MatchRule`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchRuleToml {
    pub mccmnc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imsi_prefix_xpattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gid2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iccid_prefix: Option<String>,
}

/// One module entry in the carrier's confseq.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleToml {
    pub revision: String,
    pub compressed: bool,
    /// NV items as an **insertion-order-preserving** map (key = name from `NvTable`, or
    /// `unknown_<id>` if unnamed). Order is kept so a re-encoded module reproduces the
    /// original NV-item order byte-for-byte (the modem itself is order-agnostic; this is
    /// purely for reproducibility). Each value is the item's `Val` groups — one inner
    /// list per `Val` submessage, an empty inner list being an empty `Val`. In TOML:
    /// `NAME = [[1, 2], [3]]`. (A confseq with duplicate NV-item names can't be a unique-
    /// key map, so decompile stores those verbatim instead of emitting an editable module.)
    pub items: IndexMap<String, Vec<Vec<i64>>>,
}

/// Top-level TOML document for a single carrier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CarrierFile {
    pub carrier_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    pub matching: Vec<MatchRuleToml>,
    pub confseq: BTreeMap<String, ModuleToml>,
}

impl CarrierFile {
    /// Write `self` as TOML to `path`.
    pub fn write(&self, path: &Path) -> Result<()> {
        let s = toml::to_string(self).map_err(|e| Error::Project(e.to_string()))?;
        std::fs::write(path, s).map_err(Error::from)
    }

    /// Read a `CarrierFile` from a TOML file at `path`.
    pub fn read(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)?;
        toml::from_str(&s).map_err(|e| Error::Project(e.to_string()))
    }
}

/// Project-level metadata (stored in `meta.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub release_label: String,
    pub build_info: String,
    /// `(name, version)` pairs from the `versions` table.
    pub versions: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regional_fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_parent: Option<String>,
    pub nv_table: String,
    pub locks: Vec<Lock>,
}

impl Meta {
    /// Write `self` as TOML to `path`.
    pub fn write(&self, path: &Path) -> Result<()> {
        let s = toml::to_string(self).map_err(|e| Error::Project(e.to_string()))?;
        std::fs::write(path, s).map_err(Error::from)
    }

    /// Read a `Meta` from a TOML file at `path`.
    pub fn read(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)?;
        toml::from_str(&s).map_err(|e| Error::Project(e.to_string()))
    }
}

/// A single lock entry in `meta.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lock {
    pub carrier: String,
    pub module: String,
    pub orig_hash: String,
}

/// Return the human-readable key for NV item `id`: name from `table`, or `unknown_<id>`.
pub fn item_key(table: &NvTable, id: u32) -> String {
    table
        .name(id)
        .map_or_else(|| format!("unknown_{id}"), str::to_string)
}

/// Return the numeric NV item id for `key`.
/// Parses `unknown_<u32>` literally; otherwise computes `crc32_id(key)`.
pub fn key_to_id(key: &str) -> u32 {
    if let Some(suffix) = key.strip_prefix("unknown_")
        && let Ok(id) = suffix.parse::<u32>()
    {
        return id;
    }
    crate::nvtable::crc32_id(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvtable::NvTable;

    #[test]
    fn key_roundtrip() {
        let t = NvTable::bundled();
        assert_eq!(item_key(&t, 922_505_959), "TCS_GV_OPT_CARRIER_TYPE");
        assert_eq!(key_to_id("TCS_GV_OPT_CARRIER_TYPE"), 922_505_959);
        assert_eq!(item_key(&t, 1), "unknown_1");
        assert_eq!(key_to_id("unknown_1"), 1);
    }

    #[test]
    fn carrier_toml_roundtrips() {
        let mut items = IndexMap::new();
        items.insert("TCS_GV_OPT_CARRIER_TYPE".to_string(), vec![vec![4097i64]]);
        let mut cs = std::collections::BTreeMap::new();
        cs.insert(
            "core.sim1".to_string(),
            ModuleToml {
                revision: "v1.0".into(),
                compressed: false,
                items,
            },
        );
        let cf = CarrierFile {
            carrier_id: 1839,
            parent_id: None,
            matching: vec![],
            confseq: cs,
        };
        let s = toml::to_string(&cf).unwrap();
        let back: CarrierFile = toml::from_str(&s).unwrap();
        assert_eq!(back.carrier_id, 1839);
        assert_eq!(
            back.confseq["core.sim1"].items["TCS_GV_OPT_CARRIER_TYPE"],
            vec![vec![4097]]
        );
    }
}
