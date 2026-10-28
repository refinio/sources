use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use url::Url;

#[derive(Debug, Serialize)]
pub struct Change {
    pub path: String,
    pub status: String,
}

#[derive(Debug)]
pub struct AmbiguousCommitId(pub String);

impl std::fmt::Display for AmbiguousCommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ambiguous object ID {}", self.0)
    }
}

impl std::error::Error for AmbiguousCommitId {}

pub struct Git {
    path: PathBuf,
    inside_git_dir: bool,
}

impl Git {
    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            inside_git_dir: false,
        }
    }

    pub(crate) fn for_git_dir(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            inside_git_dir: true,
        }
    }

    pub fn validate(&self, expected_url: &str, managed: bool) -> Result<()> {
        let root = self
            .path
            .canonicalize()
            .with_context(|| format!("checkout does not exist: {}", self.path.display()))?;
        let actual = self.checked(&["rev-parse", "--show-toplevel"])?;
        let actual = path_from_bytes(actual.stdout.strip_suffix(b"\n").unwrap_or(&actual.stdout));
        let actual = actual.canonicalize()?;
        if actual != root {
            bail!("{} is not a Git checkout root", self.path.display());
        }
        if managed {
            let metadata = fs::symlink_metadata(&self.path)?;
            let gitdir = self.path.join(".git");
            let git_metadata = fs::symlink_metadata(&gitdir).context(
                "managed checkout must contain a .git directory; rerun sources release --clean",
            )?;
            if metadata.file_type().is_symlink()
                || !git_metadata.is_dir()
                || git_metadata.file_type().is_symlink()
            {
                bail!(
                    "managed checkout must be an ordinary clone, not a link or linked worktree; rerun sources release --clean"
                );
            }
            let common =
                self.checked(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
            let common =
                path_from_bytes(common.stdout.strip_suffix(b"\n").unwrap_or(&common.stdout));
            if common.canonicalize()? != gitdir.canonicalize()? {
                bail!(
                    "managed Git metadata is outside the checkout; rerun sources release --clean"
                );
            }
            for item in [
                "HEAD",
                "config",
                "index",
                "objects",
                "refs",
                "shallow",
                "packed-refs",
            ] {
                let path = gitdir.join(item);
                if fs::symlink_metadata(&path).is_ok() && !path.canonicalize()?.starts_with(&root) {
                    bail!(
                        "managed Git metadata escapes the checkout at {}; rerun sources release --clean",
                        path.display()
                    );
                }
            }
            let alternates = gitdir.join("objects/info/alternates");
            if alternates.exists() && !fs::read(&alternates)?.is_empty() {
                bail!(
                    "managed checkout borrows another object store; rerun sources release --clean"
                );
            }
        }
        let expected = normalized_identity(expected_url)?;
        let remotes = if managed {
            vec!["origin".to_owned()]
        } else {
            self.text(&["remote"])?.lines().map(str::to_owned).collect()
        };
        let mut encoding_error = None;
        for remote in remotes {
            let output = self.output(&["remote", "get-url", "--all", &remote])?;
            if !output.status.success() {
                continue;
            }
            for url in output.stdout.split(|byte| *byte == b'\n') {
                let url = url.strip_suffix(b"\r").unwrap_or(url);
                if url.is_empty() {
                    continue;
                }
                let url = match std::str::from_utf8(url) {
                    Ok(url) => url,
                    Err(error) => {
                        encoding_error = Some(error);
                        continue;
                    }
                };
                let local = local_url_path(url).map(|path| self.path.join(path));
                let identity = match local {
                    Some(path) => local_identity(&path),
                    None => normalized_identity(url),
                };
                if identity.is_ok_and(|identity| identity == expected) {
                    return Ok(());
                }
            }
        }
        if let Some(error) = encoding_error {
            return Err(error).context(
                "Git returned a non-UTF-8 remote URL; configure a UTF-8 path alias or a percent-encoded file URL",
            );
        }
        bail!(
            "repository mismatch at {}: no {}remote matches {}{}",
            self.path.display(),
            if managed { "origin " } else { "" },
            redact(expected_url),
            if managed {
                "; rerun sources release --clean"
            } else {
                ""
            }
        )
    }

    pub fn branch(&self) -> Result<String> {
        let reference = self.text(&["symbolic-ref", "HEAD"]).context(
            "cannot capture branch from detached HEAD; use --ref commit or switch to a branch",
        )?;
        reference
            .strip_prefix("refs/heads/")
            .map(str::to_owned)
            .context("HEAD does not identify a local branch")
    }

    pub fn exact_tag(&self) -> Result<String> {
        let tags = self.text(&["tag", "--points-at", "HEAD"])?;
        let mut tags = tags.lines();
        let tag = tags
            .next()
            .context("HEAD has no exact tag; use --ref commit or --hint <git-ref>")?;
        if tags.next().is_some() {
            bail!("HEAD has multiple tags; choose one with --hint <git-ref>");
        }
        Ok(tag.to_owned())
    }

    pub fn head(&self) -> Result<Option<String>> {
        let output = self.output(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
        if output.status.success() {
            return Ok(Some(String::from_utf8(output.stdout)?.trim().to_owned()));
        }
        let symbolic = self.output(&["symbolic-ref", "--quiet", "HEAD"])?;
        if symbolic.status.success() {
            let reference = String::from_utf8(symbolic.stdout)?;
            let exists = self.output(&["show-ref", "--verify", "--quiet", reference.trim()])?;
            if exists.status.code() == Some(1) {
                return Ok(None);
            }
        }
        bail!(
            "cannot read checkout HEAD at {}; rerun sources release --clean",
            self.path.display()
        );
    }

    pub fn changes(&self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        self.collect_changes(Path::new(""), &mut changes)?;
        Ok(changes)
    }

    fn collect_changes(&self, prefix: &Path, changes: &mut Vec<Change>) -> Result<()> {
        let output = self.checked(&["ls-files", "--stage", "-z"])?;
        let mut modules = BTreeSet::new();
        for entry in output.stdout.split(|byte| *byte == 0) {
            if entry.starts_with(b"160000 ")
                && let Some(tab) = entry.iter().position(|byte| *byte == b'\t')
            {
                modules.insert(path_from_bytes(&entry[tab + 1..]));
            }
        }
        let output = self.checked(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
        ])?;
        let mut records = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty());
        while let Some(record) = records.next() {
            if record.len() < 4 {
                bail!("invalid Git status response at {}", self.path.display());
            }
            let status = String::from_utf8_lossy(&record[..2]).into_owned();
            let path = path_from_bytes(&record[3..]);
            if !matches!(status.as_str(), " M" | " D") || !modules.contains(&path) {
                changes.push(Change {
                    path: prefix.join(&path).to_string_lossy().into_owned(),
                    status,
                });
            }
            if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
                records.next();
            }
        }
        for module in modules {
            let path = self.path.join(&module);
            if !path.join(".git").exists() {
                continue;
            }
            let git = Git::at(&path);
            let root = git.checked(&["rev-parse", "--show-toplevel"])?;
            let root = path_from_bytes(root.stdout.strip_suffix(b"\n").unwrap_or(&root.stdout));
            if root.canonicalize()? != path.canonicalize()? {
                bail!("submodule is not a checkout root: {}", path.display());
            }
            git.collect_changes(&prefix.join(module), changes)?;
        }
        Ok(())
    }

    pub(crate) fn tag_reference(&self, tag: &str) -> Result<String> {
        let reference = format!("refs/tags/{tag}");
        if !self
            .output(&["check-ref-format", &reference])?
            .status
            .success()
        {
            bail!("invalid tag name {tag:?}; use an exact Git tag name");
        }
        Ok(reference)
    }

    pub fn resolve_tag(&self, tag: &str) -> Result<Option<String>> {
        let reference = self.tag_reference(tag)?;
        let mut resolved = None;
        for reference in [reference, cached_tag_reference(tag)] {
            let exists = self.output(&["show-ref", "--verify", "--hash", &reference])?;
            if !exists.status.success() {
                continue;
            }
            let object = String::from_utf8(exists.stdout)?;
            let commit = self
                .text(&[
                    "rev-parse",
                    "--verify",
                    &format!("{}^{{commit}}", object.trim()),
                ])
                .with_context(|| format!("tag {tag:?} does not identify a commit"))?;
            if let Some(previous) = &resolved
                && previous != &commit
            {
                bail!("ambiguous local tag {tag:?}; use --check to select the upstream tag");
            }
            resolved = Some(commit);
        }
        Ok(resolved)
    }

    pub fn resolve_commit(&self, input: &str) -> Result<String> {
        let oid = self.resolve_object(input)?;
        if self.text(&["cat-file", "-t", &oid])? != "commit" {
            bail!("{input} does not identify a commit object");
        }
        Ok(oid)
    }

    pub fn resolve_object(&self, input: &str) -> Result<String> {
        if input.len() < 4
            || input.len() > 64
            || !input.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("commit must be a full or unambiguous abbreviated hexadecimal object ID");
        }
        let matches = self.text(&[
            "rev-parse",
            &format!("--disambiguate={}", input.to_ascii_lowercase()),
        ])?;
        if matches.lines().count() > 1 {
            return Err(AmbiguousCommitId(input.to_owned()).into());
        }
        matches
            .lines()
            .next()
            .map(str::to_owned)
            .with_context(|| format!("{input} does not identify a locally available object"))
    }

    pub(crate) fn text(&self, args: &[impl AsRef<OsStr>]) -> Result<String> {
        let output = self.checked(args)?;
        Ok(String::from_utf8(output.stdout)
            .context("Git returned non-UTF-8 output")?
            .trim_end_matches(['\r', '\n'])
            .to_owned())
    }

    pub(crate) fn checked(&self, args: &[impl AsRef<OsStr>]) -> Result<Output> {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(
                "Git {} failed at {}: {}",
                args.first()
                    .map(|arg| arg.as_ref().to_string_lossy())
                    .unwrap_or_else(|| "operation".into()),
                self.path.display(),
                redact(String::from_utf8_lossy(&output.stderr).trim())
            );
        }
        Ok(output)
    }

    pub(crate) fn output(&self, args: &[impl AsRef<OsStr>]) -> Result<Output> {
        self.command_output(args, false)
    }

    pub(crate) fn network_output(&self, args: &[impl AsRef<OsStr>]) -> Result<Output> {
        self.command_output(args, true)
    }

    fn command_output(&self, args: &[impl AsRef<OsStr>], network: bool) -> Result<Output> {
        let mut command = Command::new("git");
        command.current_dir(&self.path);
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_PREFIX",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_SHALLOW_FILE",
            "GIT_REPLACE_REF_BASE",
            "GIT_GRAFT_FILE",
            "GIT_QUARANTINE_PATH",
        ] {
            command.env_remove(name);
        }
        command
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C");
        if !network {
            command.args(["-c", "protocol.allow=never"]);
        }
        if self.inside_git_dir {
            // The process already runs inside the metadata directory. Relative
            // arguments also avoid Windows verbatim paths that Git rejects.
            command.args(["--git-dir=.", "--work-tree=."]);
        }
        command.args(args);
        command.output().with_context(|| {
            format!(
                "cannot run Git at {}; install Git and verify the checkout path",
                self.path.display()
            )
        })
    }
}

pub(crate) fn cached_tag_reference(tag: &str) -> String {
    // Bound both component and total path length, without tag-prefix conflicts.
    let key = Sha256::digest(tag.as_bytes());
    format!("refs/sources/tags/{key:x}")
}

pub fn redact(text: &str) -> String {
    let mut redacted = text.to_owned();
    let mut start = 0;
    while let Some(separator) = redacted[start..].find("://") {
        let authority = start + separator + 3;
        let end = redacted[authority..]
            .find(['/', '?', '#'])
            .map_or(redacted.len(), |end| authority + end);
        if let Some(at) = redacted[authority..end].rfind('@') {
            let at = authority + at;
            redacted.replace_range(authority..at, "[redacted]");
            start = authority + "[redacted]".len() + 1;
        } else {
            start = redacted[authority..end]
                .find(char::is_whitespace)
                .map_or(end, |offset| authority + offset);
        }
    }
    redacted
}

pub fn normalized_identity(value: &str) -> Result<String> {
    if let Some((authority, path)) = value.split_once(':')
        && !value.contains("://")
        && !authority.contains(['/', '\\'])
        && authority.len() > 1
    {
        let host = authority.rsplit('@').next().unwrap_or(authority);
        return network_identity(&Url::parse(&format!("ssh://{host}/{path}"))?);
    }
    if value.contains("://") {
        let url = Url::parse(value).context("invalid repository URL")?;
        if url.scheme() == "file" {
            let path = url
                .to_file_path()
                .map_err(|()| anyhow::anyhow!("invalid file repository URL"))?;
            return local_identity(&path);
        }
        return network_identity(&url);
    }
    local_identity(Path::new(value))
}

fn local_url_path(value: &str) -> Option<&Path> {
    if value.contains("://")
        || value
            .split_once(':')
            .is_some_and(|(authority, _)| !authority.contains(['/', '\\']) && authority.len() > 1)
    {
        None
    } else {
        Some(Path::new(value))
    }
}

#[cfg(unix)]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes.to_owned()).into()
}

#[cfg(not(unix))]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    String::from_utf8_lossy(bytes).into_owned().into()
}

fn network_identity(url: &Url) -> Result<String> {
    if !matches!(url.scheme(), "https" | "http" | "ssh" | "git") {
        bail!("unsupported repository URL scheme");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("repository URLs with queries or fragments are unsupported");
    }
    let host = url
        .host()
        .context("repository URL has no host")?
        .to_string()
        .to_ascii_lowercase();
    let path = url.path().trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        bail!("repository URL has no repository path");
    }
    let port = url
        .port()
        .filter(|port| !matches!((url.scheme(), *port), ("ssh", 22) | ("git", 9418)));
    match port {
        Some(port) => Ok(format!("{}://{host}:{port}/{path}", url.scheme())),
        None => Ok(format!("{host}/{path}")),
    }
}

fn local_identity(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let absolute = crate::config::upstream_path(&absolute)?;
    Ok(Url::from_file_path(&absolute)
        .map_err(|()| anyhow::anyhow!("cannot represent repository path as a file URL"))?
        .to_string())
}
