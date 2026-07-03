//! `magisk` — package a compiled cfgdb directory into a flashable Magisk module (.zip).

use anyhow::{Context, bail};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const UPDATE_BINARY: &str = include_str!("assets/update-binary");
const UPDATER_SCRIPT: &str = "#MAGISK\n";
const DEFAULT_NAME: &str = "Pixel carrierconfig override";

/// Shared zip entry options: stored (uncompressed), with the given unix mode.
fn opts(mode: u32) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(mode)
}

/// Validate an absolute on-device directory and return it without its leading `/`
/// (and without a trailing `/`).
/// `/vendor/firmware/carrierconfig` -> `vendor/firmware/carrierconfig`.
fn dest_prefix(dest: &str) -> anyhow::Result<String> {
    let trimmed = dest
        .strip_prefix('/')
        .with_context(|| format!("--dest must be an absolute path, got {dest:?}"))?
        .trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("--dest must name a directory, not the filesystem root");
    }
    Ok(trimmed.to_string())
}

/// Build the zip entry name from a dest prefix and a slice of relative path components.
/// Produces `system/<prefix>/<comp1>/<comp2>/...` using forward slashes regardless of OS.
fn module_path(prefix: &str, rel_components: &[String]) -> String {
    let rel_str = rel_components.join("/");
    format!("system/{prefix}/{rel_str}")
}

/// Render `module.prop` for the given on-device dest, module name, and file count.
fn module_prop(dest: &str, name: &str, n_files: usize) -> String {
    format!(
        "id=pixel_carrierconfig_override\n\
         name={name}\n\
         version=v1.0\n\
         versionCode=1\n\
         author=pixel-carrierconfig-toolbox\n\
         description=Overlays the carrierconfig (cfgdb) tree onto {dest} ({n_files} files).\n",
    )
}

/// Walk `dir` recursively and collect relative path components of all regular files.
/// `rel_prefix` holds the current path components relative to the root dir.
fn walk_files(dir: &Path, rel_prefix: &[String], out: &mut Vec<Vec<String>>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading dir {}", dir.display()))? {
        let entry = entry.with_context(|| format!("reading dir entry in {}", dir.display()))?;
        let ft = entry
            .file_type()
            .with_context(|| format!("getting file type for {:?}", entry.path()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|n| anyhow::anyhow!("non-UTF-8 filename: {n:?}"))?;
        let mut components = rel_prefix.to_vec();
        components.push(name);
        if ft.is_file() {
            out.push(components);
        } else if ft.is_dir() {
            walk_files(&entry.path(), &components, out)?;
        }
        // skip symlinks and other special file types
    }
    Ok(())
}

/// Build the module `.zip` in memory from an already-compiled cfgdb directory.
///
/// - `dest`: absolute on-device path (default `/vendor/firmware/carrierconfig`).
/// - `name`: module display name; `None` → `"Pixel carrierconfig override"`.
pub fn build_module(cfgdb_dir: &Path, dest: &str, name: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let prefix = dest_prefix(dest)?;
    let name = name.unwrap_or(DEFAULT_NAME);

    // Walk the directory recursively, collecting relative path component vectors.
    let mut rel_paths: Vec<Vec<String>> = Vec::new();
    walk_files(cfgdb_dir, &[], &mut rel_paths)?;
    // Sort for deterministic output.
    rel_paths.sort();

    let n_files = rel_paths.len();

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));

    zip.start_file("module.prop", opts(0o644))?;
    zip.write_all(module_prop(dest, name, n_files).as_bytes())?;

    zip.start_file("META-INF/com/google/android/update-binary", opts(0o755))?;
    zip.write_all(UPDATE_BINARY.as_bytes())?;

    zip.start_file("META-INF/com/google/android/updater-script", opts(0o644))?;
    zip.write_all(UPDATER_SCRIPT.as_bytes())?;

    for rel in &rel_paths {
        let entry_name = module_path(&prefix, rel);
        let abs_path = rel.iter().fold(PathBuf::from(cfgdb_dir), |p, c| p.join(c));
        let data = std::fs::read(&abs_path)
            .with_context(|| format!("reading file {}", abs_path.display()))?;
        zip.start_file(entry_name, opts(0o644))?;
        zip.write_all(&data)?;
    }

    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use zip::ZipArchive;

    /// Create a fresh temp directory for a single test, unique per call.
    fn tempdir_like() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "carrierconfig-magisk-{}-{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Read a produced zip back into a list of entry names (hermetic; no system `unzip`).
    fn entry_names(zip: &[u8]) -> Vec<String> {
        let mut archive = ZipArchive::new(Cursor::new(zip.to_vec())).unwrap();
        (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect()
    }

    /// Read a produced zip back into a name -> bytes map.
    fn entries(zip: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut archive = ZipArchive::new(Cursor::new(zip.to_vec())).unwrap();
        let mut out = std::collections::BTreeMap::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            out.insert(name, buf);
        }
        out
    }

    #[test]
    fn module_overlays_nested_tree_under_dest() {
        let dir = tempdir_like();
        std::fs::create_dir_all(dir.join("confseqs")).unwrap();
        std::fs::write(dir.join("cfg.db"), b"db").unwrap();
        std::fs::write(dir.join("confseqs/abcd"), b"cs").unwrap();
        let zip = build_module(&dir, "/vendor/firmware/carrierconfig", None).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let names = entry_names(&zip);
        assert!(names.iter().any(|n| n == "module.prop"));
        assert!(
            names
                .iter()
                .any(|n| n == "META-INF/com/google/android/update-binary")
        );
        assert!(
            names
                .iter()
                .any(|n| n == "system/vendor/firmware/carrierconfig/cfg.db")
        );
        assert!(
            names
                .iter()
                .any(|n| n == "system/vendor/firmware/carrierconfig/confseqs/abcd")
        );
    }

    #[test]
    fn updater_script_is_magisk() {
        let dir = tempdir_like();
        std::fs::write(dir.join("x"), b"x").unwrap();
        let zip = build_module(&dir, "/vendor/firmware/carrierconfig", None).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let e = entries(&zip);
        assert_eq!(
            e.get("META-INF/com/google/android/updater-script").unwrap(),
            b"#MAGISK\n"
        );
    }

    #[test]
    fn module_prop_has_required_fields() {
        let dir = tempdir_like();
        std::fs::write(dir.join("x"), b"x").unwrap();
        let zip = build_module(&dir, "/vendor/firmware/carrierconfig", Some("My Mod")).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let e = entries(&zip);
        let prop = std::str::from_utf8(e.get("module.prop").unwrap()).unwrap();
        assert!(prop.contains("id=pixel_carrierconfig_override\n"));
        assert!(prop.contains("name=My Mod\n"));
        assert!(prop.contains("author=pixel-carrierconfig-toolbox\n"));
    }

    #[test]
    fn dest_override_changes_prefix() {
        let dir = tempdir_like();
        std::fs::write(dir.join("x"), b"x").unwrap();
        let zip = build_module(&dir, "/system/etc/foo/", None).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        // leading slash stripped, trailing slash trimmed, `system/` prefixed (hence system/system/…).
        assert!(
            entry_names(&zip)
                .iter()
                .any(|n| n == "system/system/etc/foo/x")
        );
    }

    #[test]
    fn non_absolute_dest_errors() {
        let dir = tempdir_like();
        let result = build_module(&dir, "vendor/firmware/carrierconfig", None);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err());
    }

    #[test]
    fn root_dest_errors() {
        let dir = tempdir_like();
        let result = build_module(&dir, "/", None);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err());
    }

    #[test]
    fn dest_prefix_strips_slashes() {
        assert_eq!(
            dest_prefix("/vendor/firmware/carrierconfig").unwrap(),
            "vendor/firmware/carrierconfig"
        );
        assert_eq!(dest_prefix("/system/etc/foo/").unwrap(), "system/etc/foo");
    }

    #[test]
    fn non_absolute_dest_prefix_errors() {
        assert!(dest_prefix("vendor/firmware/carrierconfig").is_err());
    }

    #[test]
    fn root_dest_prefix_errors() {
        assert!(dest_prefix("/").is_err());
    }

    #[test]
    fn update_binary_is_well_formed() {
        assert!(UPDATE_BINARY.starts_with("#!"));
        assert!(UPDATE_BINARY.contains("util_functions.sh"));
        assert!(UPDATE_BINARY.contains("install_module"));
    }
}
