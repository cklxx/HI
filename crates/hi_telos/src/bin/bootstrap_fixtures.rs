use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hi_telos::fixtures;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let target = if let Some(path) = args.get(1) {
        PathBuf::from(path)
    } else {
        env::current_dir().context("resolving current directory")?
    };

    let installed = fixtures::install_core_fixture(&target)?;
    println!(
        "Core fixture installed at {:?}. Set HI_APP_ROOT to this path before running the orchestrator.",
        installed
    );
    Ok(())
}
