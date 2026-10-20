use crate::git::{Git, redact};
use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    EnsureCached,
    Refresh,
    Offline,
}

pub struct Revision {
    pub commit: String,
    pub modules: Vec<ModuleRevision>,
}

pub struct ModuleRevision {
    pub name: String,
    pub path: PathBuf,
    pub url: String,
    pub revision: Revision,
}

pub fn ensure_revision(
    checkout: &Path,
    upstream: &str,
    commit: Option<&str>,
    policy: Acquisition,
    scratch: &Path,
) -> Result<Revision> {
    ensure!(
        commit.is_some() || policy == Acquisition::Refresh,
        "a locked commit is required for release readiness"
    );
    let commit = match commit {
        Some(commit) => {
            ensure!(
                (commit.len() == 40 || commit.len() == 64)
                    && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "expected a full hexadecimal commit object ID"
            );
            commit.to_ascii_lowercase()
        }
        None => {
            let parent = checkout
                .ancestors()
                .find(|path| path.is_dir())
                .context("managed state has no existing parent directory")?;
            let output = Git::at(parent).network_output(&[
                "ls-remote",
                "--symref",
                "--",
                upstream,
                "HEAD",
            ])?;
            ensure!(
                output.status.success(),
                "cannot discover upstream HEAD: {}",
                redact(String::from_utf8_lossy(&output.stderr).trim())
            );
            String::from_utf8(output.stdout)?
                .lines()
                .find_map(|line| {
                    line.strip_suffix("\tHEAD")
                        .filter(|oid| !oid.starts_with("ref:"))
                        .map(str::to_owned)
                })
                .context("upstream has no default-branch commit")?
        }
    };
    let managed_root = checkout
        .parent()
        .and_then(Path::parent)
        .context("invalid managed checkout path")?;
    safe_managed_path(checkout, managed_root)?;
    let git = Git::at(checkout);
    if !checkout.join(".git").exists() {
        ensure!(
            policy != Acquisition::Offline,
            "missing managed cache at {}; run sources fetch",
            checkout.display()
        );
        if checkout.exists() {
            ensure!(
                fs::read_dir(checkout)?.next().is_none(),
                "broken managed checkout at {}; rerun sources release --clean",
                checkout.display()
            );
        }
        fs::create_dir_all(checkout)?;
        let format = if commit.len() == 64 {
            "--object-format=sha256"
        } else {
            "--object-format=sha1"
        };
        git.checked(&["init", "--quiet", format])?;
        git.checked(&["remote", "add", "origin", upstream])?;
    }
    git.validate(upstream, true)?;
    validate_gitdir(&checkout.join(".git"), managed_root)?;
    cache_commit(&git, upstream, &commit, policy)?;
    let gitdir = checkout.join(".git");
    inspect_revision(&gitdir, upstream, &commit, policy, scratch, managed_root)
}

fn safe_relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(value) if value != ".git")),
        "unsafe submodule path or name {}",
        path.display()
    );
    Ok(())
}

fn safe_managed_path(path: &Path, root: &Path) -> Result<()> {
    let resolved_root = root.canonicalize()?;
    let resolved = crate::config::resolved_path(path)?;
    ensure!(
        resolved.starts_with(&resolved_root),
        "managed path escapes .sources at {}; rerun sources release --clean",
        path.display()
    );
    let relative = path
        .strip_prefix(root)
        .context("managed path is outside .sources")?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&cursor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "managed path is a link at {}; rerun sources release --clean",
                cursor.display()
            );
        }
    }
    Ok(())
}

fn validate_gitdir(gitdir: &Path, root: &Path) -> Result<()> {
    safe_managed_path(gitdir, root)?;
    for entry in [
        "HEAD",
        "FETCH_HEAD",
        "ORIG_HEAD",
        "config",
        "index",
        "objects",
        "objects/pack",
        "objects/info",
        "refs",
        "refs/sources/cache",
        "refs/remotes/origin",
        "shallow",
        "packed-refs",
        "objects/info/alternates",
    ] {
        safe_managed_path(&gitdir.join(entry), root)?;
    }
    let git = Git::for_git_dir(gitdir);
    let common = git.text(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    ensure!(
        Path::new(&common).canonicalize()? == gitdir.canonicalize()?,
        "submodule has external Git metadata; rerun sources release --clean"
    );
    let alternates = gitdir.join("objects/info/alternates");
    ensure!(
        !alternates.exists() || fs::read(alternates)?.is_empty(),
        "submodule borrows another object store; rerun sources release --clean"
    );
    Ok(())
}

fn inspect_revision(
    gitdir: &Path,
    upstream: &str,
    commit: &str,
    policy: Acquisition,
    scratch: &Path,
    root: &Path,
) -> Result<Revision> {
    validate_gitdir(gitdir, root)?;
    let git = Git::for_git_dir(gitdir);
    let tree = git.checked(&["ls-tree", "-r", "-z", "--full-tree", commit])?;
    let mut gitlinks = BTreeMap::new();
    for record in tree
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| record.starts_with(b"160000 "))
    {
        let record = std::str::from_utf8(record).context("submodule paths must be UTF-8")?;
        let (metadata, path) = record.split_once('\t').context("invalid Git tree record")?;
        let oid = metadata
            .split_whitespace()
            .nth(2)
            .context("missing gitlink object ID")?;
        safe_relative(Path::new(path))?;
        gitlinks.insert(path.to_owned(), oid.to_owned());
    }
    if gitlinks.is_empty() {
        return Ok(Revision {
            commit: commit.to_owned(),
            modules: Vec::new(),
        });
    }
    safe_managed_path(scratch, root)?;
    fs::create_dir_all(scratch)?;
    let view = tempfile::Builder::new()
        .prefix("inspect-")
        .tempdir_in(scratch)?;
    let view_git = Git::at(view.path());
    let format = if commit.len() == 64 {
        "--object-format=sha256"
    } else {
        "--object-format=sha1"
    };
    view_git.checked(&["init", "--quiet", format])?;
    view_git.checked(&["remote", "add", "origin", upstream])?;
    view_git.checked(&[
        "-c",
        "protocol.file.allow=always",
        "fetch",
        "--no-tags",
        "--depth=1",
        "--recurse-submodules=no",
        gitdir.to_str().context("managed paths must be UTF-8")?,
        commit,
    ])?;
    view_git.checked(&["checkout", "--detach", "--no-overwrite-ignore", commit])?;
    view_git.checked(&["submodule", "init"])?;
    let paths = view_git.checked(&[
        "config",
        "--null",
        "--file",
        ".gitmodules",
        "--get-regexp",
        "^submodule\\..*\\.path$",
    ])?;
    let mut modules = Vec::new();
    for record in paths
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record =
            std::str::from_utf8(record).context("submodule configuration must be UTF-8")?;
        let (key, path) = record
            .split_once('\n')
            .context("invalid submodule configuration")?;
        let name = key
            .strip_prefix("submodule.")
            .and_then(|name| name.strip_suffix(".path"))
            .context("invalid submodule name")?;
        let Some(module_commit) = gitlinks.remove(path) else {
            continue;
        };
        safe_relative(Path::new(name))?;
        let url = view_git.text(&["config", "--get", &format!("submodule.{name}.url")])?;
        let module_gitdir = gitdir.join("modules").join(name);
        safe_managed_path(&module_gitdir, root)?;
        let result = (|| -> Result<Revision> {
            if !module_gitdir.exists() {
                ensure!(
                    policy != Acquisition::Offline,
                    "missing cached submodule inputs; run sources fetch"
                );
                fs::create_dir_all(&module_gitdir)?;
                let initial = Git::at(&module_gitdir);
                let format = if module_commit.len() == 64 {
                    "--object-format=sha256"
                } else {
                    "--object-format=sha1"
                };
                initial.checked(&["init", "--bare", "--quiet", format])?;
                let module = Git::for_git_dir(&module_gitdir);
                module.checked(&["config", "core.bare", "false"])?;
                module.checked(&["remote", "add", "origin", &url])?;
            }
            validate_gitdir(&module_gitdir, root)?;
            let module = Git::for_git_dir(&module_gitdir);
            cache_commit(&module, &url, &module_commit, policy)?;
            inspect_revision(&module_gitdir, &url, &module_commit, policy, scratch, root)
        })()
        .with_context(|| format!("submodule {path} at {module_commit}"))?;
        modules.push(ModuleRevision {
            name: name.to_owned(),
            path: PathBuf::from(path),
            url,
            revision: result,
        });
    }
    ensure!(
        gitlinks.is_empty(),
        "gitlink has no .gitmodules entry: {}",
        gitlinks.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Revision {
        commit: commit.to_owned(),
        modules,
    })
}

fn complete_tree(git: &Git, commit: &str) -> Result<bool> {
    let kind = git.output(&["cat-file", "-t", commit])?;
    if !kind.status.success() || kind.stdout != b"commit\n" {
        return Ok(false);
    }
    let tree = format!("{commit}^{{tree}}");
    let objects = git.output(&[
        "rev-list",
        "--objects",
        "--missing=print",
        "--no-object-names",
        &tree,
    ])?;
    Ok(objects.status.success()
        && !objects
            .stdout
            .split(|byte| *byte == b'\n')
            .any(|line| line.starts_with(b"?")))
}

fn cache_commit(git: &Git, upstream: &str, commit: &str, policy: Acquisition) -> Result<()> {
    let complete = complete_tree(git, commit)?;
    if policy == Acquisition::Offline {
        ensure!(
            complete,
            "missing cached commit, tree, or file contents for {commit}; run sources fetch"
        );
        return Ok(());
    }
    if policy == Acquisition::Refresh {
        let contact = git.network_output(&["ls-remote", "--", upstream])?;
        ensure!(
            contact.status.success(),
            "cannot contact configured upstream: {}",
            redact(String::from_utf8_lossy(&contact.stderr).trim())
        );
    }
    if !complete || policy == Acquisition::Refresh {
        let mut args = vec!["fetch", "--no-tags", "--depth=1", "--recurse-submodules=no"];
        if !complete && git.output(&["cat-file", "-e", commit])?.status.success() {
            args.push("--refetch");
        }
        args.extend(["--", upstream, commit]);
        let output = git.network_output(&args)?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            let semantic = [
                "not our ref",
                "unadvertised object",
                "couldn't find remote ref",
                "does not support shallow",
                "shallow requests are not supported",
                "shallow fetching is not supported",
            ]
            .iter()
            .any(|message| error.contains(message));
            ensure!(
                semantic,
                "cannot fetch required commit {commit}: {}",
                redact(error.trim())
            );
            if !complete {
                compatibility_fetch(git, upstream, commit, error.contains("shallow"))?;
            }
        }
    }
    ensure!(
        complete_tree(git, commit)?,
        "incomplete cached inputs for commit {commit}; rerun sources fetch or sources release --clean"
    );
    git.checked(&[
        "update-ref",
        &format!("refs/sources/cache/{commit}"),
        commit,
    ])?;
    Ok(())
}

fn compatibility_fetch(git: &Git, upstream: &str, commit: &str, no_shallow: bool) -> Result<()> {
    eprintln!(
        "warning: compatibility fallback at {} for {commit}: fetching advertised refs and broader history as needed",
        redact(upstream)
    );
    let advertised = git.network_output(&["ls-remote", "--", upstream])?;
    ensure!(
        advertised.status.success(),
        "cannot list upstream refs: {}",
        redact(String::from_utf8_lossy(&advertised.stderr).trim())
    );
    let mut exact = Vec::new();
    let mut heads = Vec::new();
    let mut tags = Vec::new();
    for line in String::from_utf8(advertised.stdout)?.lines() {
        let Some((oid, name)) = line.split_once('\t') else {
            continue;
        };
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            let reference = format!("+{name}:refs/remotes/origin/{branch}");
            if oid == commit {
                exact.push(reference);
            } else {
                heads.push(reference);
            }
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            let tag = tag.strip_suffix("^{}").unwrap_or(tag);
            let reference = format!("refs/tags/{tag}:refs/sources/advertised/{commit}/tags/{tag}");
            if oid == commit {
                exact.push(reference);
            } else {
                tags.push(reference);
            }
        }
    }
    exact.sort();
    exact.dedup();
    tags.sort();
    tags.dedup();
    tags.retain(|reference| exact.binary_search(reference).is_err());
    ensure!(
        !exact.is_empty() || !heads.is_empty() || !tags.is_empty(),
        "upstream has no advertised refs supplying required commit {commit}"
    );
    let candidates = [exact, heads, tags];
    for depth in [1, 16, 64, 256, 1024, 0] {
        if no_shallow && depth != 0 {
            continue;
        }
        for refs in &candidates {
            if refs.is_empty() {
                continue;
            }
            let mut args = vec![
                "fetch".to_owned(),
                "--no-tags".to_owned(),
                "--recurse-submodules=no".to_owned(),
            ];
            if depth != 0 {
                args.push(format!("--depth={depth}"));
            } else if git.text(&["rev-parse", "--is-shallow-repository"])? == "true" {
                args.push("--unshallow".to_owned());
            }
            args.push("--".to_owned());
            args.push(upstream.to_owned());
            args.extend(refs.iter().cloned());
            let output = git.network_output(&args)?;
            ensure!(
                output.status.success(),
                "compatibility fetch failed for {commit}: {}",
                redact(String::from_utf8_lossy(&output.stderr).trim())
            );
            if complete_tree(git, commit)? {
                return Ok(());
            }
        }
    }
    bail!("upstream cannot supply required commit {commit}")
}
