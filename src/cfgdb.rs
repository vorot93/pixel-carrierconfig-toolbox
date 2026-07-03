//! cfgdb — read-only access to `cfg.db` (carrier configuration database).

use rusqlite::{Connection, OpenFlags};
use std::path::Path;

use crate::error::{Error, Result};

/// A single SIM-identity matching rule from `carrier_info`.
pub struct MatchRule {
    pub mccmnc: String,
    pub imsi_prefix_xpattern: Option<String>,
    pub spn: Option<String>,
    pub gid1: Option<String>,
    pub gid2: Option<String>,
    pub iccid_prefix: Option<String>,
}

/// A carrier entry assembled from `confnames`/`carrier_name`/`confmap`/`carrier_info`.
pub struct Carrier {
    pub carrier_id: i64,
    pub slug: Option<String>,
    pub name: Option<String>,
    pub parent_id: Option<i64>,
    pub confman: String,
    pub matching: Vec<MatchRule>,
}

/// In-memory representation of `cfg.db`.
pub struct Cfgdb {
    pub carriers: Vec<Carrier>,
    /// `(name, version)` rows from the `versions` table.  Integer versions are stored as their
    /// decimal string representation; the `confpack` TEXT label is preserved as-is.
    pub versions: Vec<(String, String)>,
    /// Open read-only connection retained for `confman_confseqs` queries.
    conn: Connection,
}

/// Normalise a refiner column value: treat SQL NULL, empty string, or the catch-all `%` as
/// "no filter" (`None`).
fn normalize_refiner(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty() && v != "%")
}

impl Cfgdb {
    /// Open `dir/cfg.db` read-only and load all carrier data.
    pub fn read(dir: &Path) -> Result<Cfgdb> {
        let db_path = dir.join("cfg.db");
        let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        // --- versions -------------------------------------------------------
        let versions = {
            let mut stmt = conn.prepare("SELECT name, version FROM versions")?;
            let rows = stmt.query_map([], |row| {
                let name: String = row.get(0)?;
                // `version` is INTEGER-declared but the `confpack` row stores a TEXT label.
                // Read via Value to preserve each type faithfully.
                let version: String = match row.get::<_, rusqlite::types::Value>(1)? {
                    rusqlite::types::Value::Integer(i) => i.to_string(),
                    rusqlite::types::Value::Text(s) => s,
                    rusqlite::types::Value::Real(f) => f.to_string(),
                    _ => String::new(),
                };
                Ok((name, version))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        // --- carriers (confmap ⟕ confnames ⟕ carrier_name) ------------------
        // confmap.carrier_id is declared TEXT; CAST to INTEGER for the join.
        // Collect raw rows first so parent_id parsing can return our Error type.
        let mut carriers: Vec<Carrier> = {
            let mut stmt = conn.prepare(
                "SELECT CAST(cm.carrier_id AS INTEGER) AS cid,
                        cn.name  AS slug,
                        cna.name AS display_name,
                        cm.parent_id,
                        cm.confman
                 FROM confmap cm
                 LEFT JOIN confnames    cn  ON cn.carrier_id  = CAST(cm.carrier_id AS INTEGER)
                 LEFT JOIN carrier_name cna ON cna.carrier_id = CAST(cm.carrier_id AS INTEGER)",
            )?;
            // Collect as raw tuples; parent_id stays as Option<String> for strict parsing below.
            #[allow(clippy::type_complexity)]
            let raw: Vec<(
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            )> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            // Parse parent_id strictly: empty string → None, non-empty non-numeric → Err.
            raw.into_iter()
                .map(|(carrier_id, slug, name, pid_raw, confman)| {
                    let parent_id = match pid_raw {
                        None => None,
                        Some(s) if s.is_empty() => None,
                        Some(s) => Some(s.parse::<i64>().map_err(|_| {
                            Error::Project(format!(
                                "carrier_id {carrier_id}: non-numeric parent_id {s:?}"
                            ))
                        })?),
                    };
                    Ok(Carrier {
                        carrier_id,
                        slug,
                        name,
                        parent_id,
                        confman,
                        matching: Vec::new(),
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };

        // --- matching rules (carrier_info) -----------------------------------
        {
            let mut stmt = conn.prepare(
                "SELECT mccmnc, imsi_prefix_xpattern, spn, gid1, gid2, iccid_prefix
                 FROM carrier_info WHERE carrier_id = ?1",
            )?;
            for carrier in &mut carriers {
                let rules = stmt.query_map([carrier.carrier_id], |row| {
                    let imsi: Option<String> = row.get(1)?;
                    let spn: Option<String> = row.get(2)?;
                    let gid1: Option<String> = row.get(3)?;
                    let gid2: Option<String> = row.get(4)?;
                    let iccid: Option<String> = row.get(5)?;
                    Ok(MatchRule {
                        mccmnc: row.get(0)?,
                        imsi_prefix_xpattern: normalize_refiner(imsi),
                        spn: normalize_refiner(spn),
                        gid1: normalize_refiner(gid1),
                        gid2: normalize_refiner(gid2),
                        iccid_prefix: normalize_refiner(iccid),
                    })
                })?;
                for rule in rules {
                    carrier.matching.push(rule?);
                }
            }
        }

        Ok(Cfgdb {
            carriers,
            versions,
            conn,
        })
    }

    /// Return the `confseq` column from `confman_<confman>`, preserving row order and duplicates.
    ///
    /// # Errors
    /// Returns [`Error::Project`] if `confman` is not exactly 40 lowercase hex characters
    /// (prevents SQL injection via table-name interpolation).
    pub fn confman_confseqs(&self, confman: &str) -> Result<Vec<String>> {
        if confman.len() != 40
            || !confman
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(Error::Project(format!(
                "confman must be exactly 40 lowercase hex chars, got: {confman:?}"
            )));
        }
        let sql = format!("SELECT confseq FROM confman_{confman}");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Option<std::path::PathBuf> {
        std::env::var_os("CFGDB_CORPUS").map(Into::into)
    }

    #[test]
    fn reads_verizon() {
        let Some(dir) = corpus() else {
            eprintln!("skip: set CFGDB_CORPUS");
            return;
        };
        let db = Cfgdb::read(&dir).unwrap();
        let vzw = db
            .carriers
            .iter()
            .find(|c| c.slug.as_deref() == Some("us_vzw"))
            .unwrap();
        assert_eq!(vzw.carrier_id, 1839);
        let seqs = db.confman_confseqs(&vzw.confman).unwrap();
        assert!(!seqs.is_empty());
        // Confirm the confpack label round-trips as a string (not coerced to 0).
        eprintln!("versions: {:?}", db.versions);
        let confpack_ver = db
            .versions
            .iter()
            .find(|(name, _)| name == "confpack")
            .map(|(_, v)| v.as_str());
        assert!(
            confpack_ver.is_some_and(|v| v.starts_with("cfgdb-")),
            "expected confpack version to start with 'cfgdb-', got: {confpack_ver:?}"
        );
    }
}
