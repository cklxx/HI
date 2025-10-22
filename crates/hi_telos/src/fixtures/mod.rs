use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const CORE_FIXTURE_DIR: &str = "tests/fixtures/core";

/// Return the on-disk location of the bundled core fixture.
pub fn core_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CORE_FIXTURE_DIR)
}

/// Install the bundled core fixture into the provided target root.
///
/// The fixture contains baseline `config/` and `data/` layouts that
/// exercise the Inbox → Beat → Agent → Journal/SP flow without external
/// dependencies. Existing files will be overwritten if they share the
/// same path.
pub fn install_core_fixture(target_root: &Path) -> Result<PathBuf> {
    let fixture_root = core_fixture_root();

    copy_dir_recursive(&fixture_root.join("config"), &target_root.join("config"))
        .with_context(|| "copying config fixture")?;
    copy_dir_recursive(&fixture_root.join("data"), &target_root.join("data"))
        .with_context(|| "copying data fixture")?;

    Ok(target_root.to_path_buf())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }

    fs::create_dir_all(dst).with_context(|| format!("creating fixture dir {:?}", dst))?;

    for entry in fs::read_dir(src).with_context(|| format!("reading fixture dir {:?}", src))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating fixture parent dir {:?}", parent))?;
            }
            fs::copy(&src_path, &dst_path)
                .with_context(|| format!("copying fixture file {:?}", src_path))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn core_fixture_root_exists() {
        let root = core_fixture_root();
        assert!(root.exists(), "core fixture directory should exist");
    }

    #[test]
    fn install_core_fixture_copies_files() {
        let tmp = TempDir::new().expect("temp dir");
        let target = tmp.path();

        let installed = install_core_fixture(target).expect("install fixture");
        assert!(installed.join("config/agent.yml").exists());
        assert!(
            installed
                .join("data/intent/inbox/20240101T000000-00000000-0000-0000-0000-000000000001.md")
                .exists()
        );
    }
}
