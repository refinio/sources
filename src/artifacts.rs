//! Fingerprint the exact file trees a consumer declares, independently of Turbo input keys.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sources::config::ContentTask;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

type Files = BTreeMap<Vec<u8>, (Vec<u8>, bool)>;

fn path_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    for component in path.components() {
        if !bytes.is_empty() {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(component.as_os_str().as_encoded_bytes());
    }
    bytes
}

pub(crate) fn snapshot(root: &Path, relative: &Path) -> Result<Files> {
    let path = root.join(relative);
    let canonical = fs::canonicalize(&path)
        .with_context(|| format!("missing required artifact {}", path.display()))?;
    // Resolve the declared root, including pnpm's package-directory links. A
    // configured virtual store may legitimately live outside the project.
    let mut files = Files::new();
    collect(&canonical, &canonical, &mut files)?;
    ensure!(
        !files.is_empty(),
        "required artifact {} contains no files",
        path.display()
    );
    Ok(files)
}

fn collect(base: &Path, path: &Path, files: &mut Files) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(base, &entry?.path(), files)?;
        }
    } else {
        ensure!(
            metadata.is_file(),
            "required artifact {} must contain ordinary files, not links or special files",
            path.display()
        );
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        let mut hash = Sha256::new();
        std::io::copy(&mut fs::File::open(path)?, &mut hash)?;
        files.insert(
            path_bytes(path.strip_prefix(base)?),
            (hash.finalize().to_vec(), executable),
        );
    }
    Ok(())
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

pub(crate) fn digest(root: &Path, content: &ContentTask) -> Result<String> {
    let mut artifacts = BTreeMap::new();
    for artifact in &content.artifacts {
        artifacts.insert(path_bytes(&artifact.path), snapshot(root, &artifact.path)?);
    }
    let mut hash = Sha256::new();
    field(&mut hash, b"sources-consumed-artifacts-v1");
    hash.update((artifacts.len() as u64).to_le_bytes());
    for (path, files) in artifacts {
        field(&mut hash, &path);
        hash.update((files.len() as u64).to_le_bytes());
        for (name, (bytes, executable)) in files {
            field(&mut hash, &name);
            field(&mut hash, &bytes);
            field(&mut hash, &[u8::from(executable)]);
        }
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}
