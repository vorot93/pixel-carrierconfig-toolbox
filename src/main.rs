// Thin binary; all modules live in the library crate (src/lib.rs).
use anyhow::Context;
use clap::{Parser, Subcommand};
use pixel_carrierconfig_toolbox::{
    magisk,
    nvtable::NvTable,
    project::{compile, decompile},
    report,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "pixel-carrierconfig-toolbox",
    about = "Decompile/recompile Pixel carrierconfig (cfgdb)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Decompile a cfgdb directory into an editable project.
    Decompile {
        cfgdb_dir: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Recompile a project back into a cfgdb directory.
    Compile {
        project_dir: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long)]
        keep_sha2: bool,
        #[arg(long)]
        magisk: Option<PathBuf>,
        #[arg(long, default_value = "/vendor/firmware/carrierconfig")]
        dest: String,
    },
    /// Package an already-compiled cfgdb directory into a Magisk module.
    Magisk {
        cfgdb_dir: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = "/vendor/firmware/carrierconfig")]
        dest: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Show one carrier's NV items.
    Inspect {
        project_dir: PathBuf,
        slug: String,
        #[arg(long)]
        full: bool,
    },
    /// Validate a project (round-trip + integrity).
    Check { project_dir: PathBuf },
    /// Built-in codec sanity checks.
    SelfTest,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::SelfTest => report::self_test(),
        Cmd::Decompile { cfgdb_dir, out } => {
            let nv = NvTable::bundled();
            decompile::decompile(&cfgdb_dir, &out, &nv)?;
            Ok(())
        }
        Cmd::Compile {
            project_dir,
            out,
            keep_sha2,
            magisk: magisk_zip,
            dest,
        } => {
            compile::compile(&project_dir, &out, keep_sha2)?;
            if let Some(zip_path) = magisk_zip {
                let z = magisk::build_module(&out, &dest, None)?;
                std::fs::write(&zip_path, &z)
                    .with_context(|| format!("writing module {}", zip_path.display()))?;
            }
            Ok(())
        }
        Cmd::Magisk {
            cfgdb_dir,
            out,
            dest,
            name,
        } => {
            let zip = magisk::build_module(&cfgdb_dir, &dest, name.as_deref())?;
            std::fs::write(&out, &zip)
                .with_context(|| format!("writing module {}", out.display()))?;
            Ok(())
        }
        Cmd::Inspect {
            project_dir,
            slug,
            full,
        } => report::inspect(&project_dir, &slug, full),
        Cmd::Check { project_dir } => report::check(&project_dir),
    }
}
