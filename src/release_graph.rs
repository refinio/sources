use crate::acquisition::{Acquisition, Revision, ensure_revision};
use crate::config::{CONFIG, Config, LOCK, Pins, Project};
use crate::git::{Git, normalized_identity};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Inspect committed metadata, never active development links or worktree files.
pub fn validate(
    project: &Project,
    revisions: &BTreeMap<String, Revision>,
    policy: Acquisition,
    registry: bool,
) -> Result<()> {
    if revisions.is_empty() {
        return Ok(());
    }
    let mut graph = Graph {
        seen: BTreeMap::new(),
        cache: project.internal().join("release-graph/checkouts"),
        scratch: project.internal().join("release-graph"),
        policy,
        registry,
    };
    let app = Git::at(&project.root);
    let inside = app.output(&["rev-parse", "--show-toplevel"])?;
    if inside.status.success() {
        let origin = app.output(&["config", "--get", "remote.origin.url"])?;
        if origin.status.success() {
            let root = crate::git::path_from_bytes(
                inside.stdout.strip_suffix(b"\n").unwrap_or(&inside.stdout),
            );
            let url = String::from_utf8(origin.stdout)?;
            let url = crate::config::resolve_url(&root, url.trim_end_matches(['\r', '\n']))?;
            if let Some(commit) = app.head()? {
                graph.enter(&url, &commit, "root")?;
            }
        } else {
            ensure!(
                origin.status.code() == Some(1),
                "cannot inspect starting repository origin"
            );
        }
    } else {
        ensure!(
            String::from_utf8_lossy(&inside.stderr).contains("not a git repository"),
            "cannot inspect starting repository: {}",
            crate::git::redact(String::from_utf8_lossy(&inside.stderr).trim())
        );
    }
    for (name, revision) in revisions {
        let source = &project.config.sources[name];
        if graph.enter(&source.url, &revision.commit, name)? {
            let checkout = project.managed(name);
            graph.walk(&checkout.join(".git"), revision, &checkout, name)?;
        }
    }
    Ok(())
}

struct Graph {
    seen: BTreeMap<String, (String, String)>,
    cache: PathBuf,
    scratch: PathBuf,
    policy: Acquisition,
    registry: bool,
}

impl Graph {
    fn enter(&mut self, url: &str, commit: &str, path: &str) -> Result<bool> {
        let identity = normalized_identity(url)?;
        let commit = commit.to_ascii_lowercase();
        if let Some((previous, previous_path)) = self.seen.get(&identity) {
            ensure!(
                previous == &commit,
                "conflicting pinned versions for repository {identity}:\n  {previous_path}: {previous}\n  {path}: {commit}\nAlign the dependency lockfiles, commit them, and update the consuming pins before retrying."
            );
            return Ok(false);
        }
        self.seen.insert(identity, (commit, path.to_owned()));
        Ok(true)
    }

    fn walk(&mut self, gitdir: &Path, revision: &Revision, owner: &Path, path: &str) -> Result<()> {
        for module in &revision.modules {
            let module_path = format!("{path} -> submodule {}", module.path.display());
            if self.enter(&module.url, &module.revision.commit, &module_path)? {
                self.walk(
                    &gitdir.join("modules").join(&module.name),
                    &module.revision,
                    &owner.join(&module.path),
                    &module_path,
                )?;
            }
        }
        let Some(bytes) = file(gitdir, &revision.commit, CONFIG)? else {
            return Ok(());
        };
        let mut config: Config = serde_json::from_slice(&bytes)
            .with_context(|| format!("{path}: invalid pinned {CONFIG}"))?;
        ensure!(
            config.version == 1,
            "{path}: unsupported pinned configuration version {}",
            config.version
        );
        config.resolve_urls(owner)?;
        let pins: Pins = match file(gitdir, &revision.commit, LOCK)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("{path}: invalid pinned {LOCK}"))?,
            None => Pins::default(),
        };
        ensure!(
            pins.version == 2,
            "{path}: unsupported pinned lock version {}",
            pins.version
        );
        for (name, source) in &config.sources {
            if self.registry && source.has_packages() {
                continue;
            }
            let commit = pins
                .sources
                .get(name)
                .and_then(|pin| pin.git.as_deref())
                .with_context(|| {
                    format!("{path} -> {name}: missing Git pin in committed {LOCK}")
                })?;
            let child_path = format!("{path} -> {name}");
            if !self.enter(&source.url, commit, &child_path)? {
                continue;
            }
            let identity = normalized_identity(&source.url)?;
            let cached = self
                .cache
                .join(format!("{:x}", Sha256::digest(identity.as_bytes())));
            let root = self.cache.parent().expect("graph cache has a managed root");
            crate::config::real_directory_if_present(root)?;
            std::fs::create_dir_all(root)?;
            let child = ensure_revision(
                &cached,
                &source.url,
                Some(commit),
                self.policy,
                &self.scratch,
            )
            .with_context(|| format!("{child_path}: cannot cache pinned dependency metadata"))?;
            self.walk(
                &cached.join(".git"),
                &child,
                &owner.join(".sources/checkouts").join(name),
                &child_path,
            )?;
        }
        Ok(())
    }
}

fn file(gitdir: &Path, commit: &str, name: &str) -> Result<Option<Vec<u8>>> {
    let git = Git::for_git_dir(gitdir);
    if git
        .checked(&["ls-tree", "--name-only", commit, "--", name])?
        .stdout
        .is_empty()
    {
        return Ok(None);
    }
    Ok(Some(
        git.checked(&["show", &format!("{commit}:{name}")])?.stdout,
    ))
}
