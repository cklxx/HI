use std::path::{Path, PathBuf};

use anyhow::Result;

pub fn install_core_fixture(root: &Path) -> Result<PathBuf> {
    hi_telos::fixtures::install_core_fixture(root)
}
