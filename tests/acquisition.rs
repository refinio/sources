mod common;

use common::{Fixture, success};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

#[test]
fn fetch_caches_exact_shallow_revisions_without_moving_existing_head() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let first = f.git(&upstream, &["rev-parse", "HEAD"]);
    std::fs::write(upstream.join("tracked.txt"), "second\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "second"]);
    let second = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.configure(json!({"a": {"url": upstream}}), json!({}));
    let lock = f.app.join(".sources.lock");
    std::fs::write(
        &lock,
        json!({"version": 1, "sources": {"a": first}}).to_string(),
    )
    .unwrap();

    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["rev-list", "--count", &first]), "1");
    assert!(managed.join(".git/shallow").exists());
    assert!(!managed.join("tracked.txt").exists());
    f.git(&managed, &["checkout", "--detach", &first]);

    std::fs::write(
        &lock,
        json!({"version": 1, "sources": {"a": second}}).to_string(),
    )
    .unwrap();
    let before_lock = std::fs::read(&lock).unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), first);
    assert_eq!(f.git(&managed, &["rev-list", "--count", &second]), "1");
    assert_eq!(
        std::fs::read_to_string(managed.join("tracked.txt")).unwrap(),
        "initial\n"
    );
    assert_eq!(std::fs::read(&lock).unwrap(), before_lock);
    assert!(!f.app.join("sources").exists());
    assert!(!managed.join(".git/objects/info/alternates").exists());
}

#[test]
fn readiness_caches_future_recursive_modules_and_changed_urls_without_switching_heads() {
    let f = Fixture::new();
    let nested = f.repo("nested");
    let nested_first = f.git(&nested, &["rev-parse", "HEAD"]);
    let old_module = f.repo("module-old");
    f.git(
        &old_module,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../nested",
            "nested child",
        ],
    );
    f.git(&old_module, &["commit", "-qam", "nested module"]);
    let old_module_pin = f.git(&old_module, &["rev-parse", "HEAD"]);
    let upstream = f.repo("upstream");
    f.git(
        &upstream,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../module-old",
            "dep",
        ],
    );
    f.git(&upstream, &["commit", "-qam", "parent module"]);
    let parent_first = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.configure(json!({"a": {"url": upstream}}), json!({}));
    let lock = f.app.join(".sources.lock");
    std::fs::write(
        &lock,
        json!({"version": 1, "sources": {"a": parent_first}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    f.git(&managed, &["checkout", "--detach", &parent_first]);
    f.git(
        &managed,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--recursive",
        ],
    );

    std::fs::write(nested.join("tracked.txt"), "future nested contents\n").unwrap();
    f.git(&nested, &["commit", "-qam", "future nested revision"]);
    let nested_next = f.git(&nested, &["rev-parse", "HEAD"]);
    let new_module = f.repo("module-new");
    f.git(
        &new_module,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../nested",
            "nested child",
        ],
    );
    f.git(&new_module, &["commit", "-qam", "replacement module"]);
    let new_module_pin = f.git(&new_module, &["rev-parse", "HEAD"]);
    f.git(
        &upstream,
        &[
            "config",
            "--file",
            ".gitmodules",
            "submodule.dep.url",
            "../module-new",
        ],
    );
    f.git(&upstream, &["add", ".gitmodules"]);
    f.git(
        &upstream,
        &[
            "update-index",
            "--cacheinfo",
            &format!("160000,{new_module_pin},dep"),
        ],
    );
    f.git(&upstream, &["commit", "-qm", "future modules"]);
    let parent_next = f.git(&upstream, &["rev-parse", "HEAD"]);
    std::fs::write(
        &lock,
        json!({"version": 1, "sources": {"a": parent_next}}).to_string(),
    )
    .unwrap();

    success(f.cli().arg("check-release").output().unwrap());
    assert_eq!(f.git(&managed, &["rev-parse", "HEAD"]), parent_first);
    assert_eq!(
        f.git(&managed.join("dep"), &["rev-parse", "HEAD"]),
        old_module_pin
    );
    assert_eq!(
        f.git(&managed.join("dep/nested child"), &["rev-parse", "HEAD"]),
        nested_first
    );
    assert_eq!(
        f.git(
            &managed,
            &[
                "--git-dir=.git/modules/dep",
                "cat-file",
                "-t",
                &new_module_pin
            ]
        ),
        "commit"
    );
    assert_eq!(
        f.git(
            &managed,
            &[
                "--git-dir=.git/modules/dep/modules/nested child",
                "cat-file",
                "-t",
                &nested_next
            ]
        ),
        "commit"
    );
    assert_eq!(
        f.git(
            &managed,
            &[
                "--git-dir=.git/modules/dep/modules/nested child",
                "rev-list",
                "--count",
                &nested_next
            ]
        ),
        "1"
    );
    assert_eq!(
        std::fs::read_to_string(managed.join("dep/nested child/tracked.txt")).unwrap(),
        "initial\n"
    );
}

#[test]
fn a_server_without_shallow_support_uses_a_reported_compatibility_fallback() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    std::fs::write(upstream.join("tracked.txt"), "newer revision\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "newer"]);
    f.git(&upstream, &["update-server-info"]);
    let server = DumbHttp::start(&upstream.join(".git"));
    f.configure(json!({"a": {"url": server.url}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    let out = success(f.cli().arg("fetch").output().unwrap());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("fallback") && stderr.contains("a"),
        "{stderr}"
    );
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["cat-file", "-t", &pin]), "commit");
    assert!(!managed.join("tracked.txt").exists());
}

#[test]
fn fallback_finds_a_pin_reachable_only_through_annotated_tag_history() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    std::fs::write(upstream.join("tracked.txt"), "tagged descendant\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "descendant"]);
    f.git(&upstream, &["tag", "-am", "release", "release"]);
    f.git(&upstream, &["update-ref", "-d", "refs/heads/main"]);
    f.git(&upstream, &["update-server-info"]);
    let server = DumbHttp::start(&upstream.join(".git"));
    f.configure(json!({"a": {"url": server.url}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(
        f.git(&managed, &["show", &format!("{pin}:tracked.txt")]),
        "initial"
    );
    assert_eq!(
        f.git(
            &managed,
            &["for-each-ref", "--format=%(refname)", "refs/tags"]
        ),
        ""
    );
    assert!(
        !f.git(
            &managed,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/sources/advertised"
            ]
        )
        .is_empty()
    );
}

#[test]
fn fallback_checks_tag_history_before_exhausting_an_unrelated_branch() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    std::fs::write(upstream.join("tracked.txt"), "tagged descendant\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "descendant"]);
    f.git(&upstream, &["tag", "-am", "source input", "source-input"]);
    f.git(&upstream, &["checkout", "--orphan", "unrelated"]);
    f.git(&upstream, &["commit", "-qm", "unrelated root"]);
    for index in 0..30 {
        f.git(
            &upstream,
            &[
                "commit",
                "--allow-empty",
                "-qm",
                &format!("unrelated {index}"),
            ],
        );
    }
    f.git(&upstream, &["update-ref", "-d", "refs/heads/main"]);
    std::fs::write(f.root.join("gitconfig"), "[protocol]\nversion = 0\n").unwrap();
    let server = f.server("upstream");
    f.configure(json!({"a": {"url": server.url}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    let output = success(f.cli().arg("fetch").output().unwrap());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fallback"));
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["cat-file", "-t", &pin]), "commit");
    let count: usize = f
        .git(
            &managed,
            &["rev-list", "--count", "refs/remotes/origin/unrelated"],
        )
        .parse()
        .unwrap();
    assert!(
        count <= 16,
        "unrelated branch acquired {count} commits despite a nearby tagged input"
    );
    assert_eq!(
        f.git(&managed, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    assert_eq!(
        f.git(
            &managed,
            &["for-each-ref", "--format=%(refname)", "refs/tags"]
        ),
        ""
    );
}

#[test]
fn fallback_prioritizes_an_advertised_tag_matching_the_required_commit() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    f.git(
        &upstream,
        &["tag", "-am", "required input", "required-input"],
    );
    f.git(&upstream, &["checkout", "--orphan", "unrelated"]);
    f.git(&upstream, &["commit", "-qm", "unrelated root"]);
    f.git(&upstream, &["update-ref", "-d", "refs/heads/main"]);
    f.git(&upstream, &["update-server-info"]);
    let server = DumbHttp::start(&upstream.join(".git"));
    f.configure(json!({"a": {"url": server.url}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["cat-file", "-t", &pin]), "commit");
    assert_eq!(
        f.git(
            &managed,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/remotes/origin/unrelated"
            ]
        ),
        ""
    );
}

#[test]
fn readiness_recovers_a_missing_working_tree_blob_from_upstream() {
    use std::process::Stdio;
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    let blob = f.git(&upstream, &["rev-parse", "HEAD:tracked.txt"]);
    f.configure(json!({"a": {"url": upstream}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 1, "sources": {"a": pin}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    let packs = std::fs::read_dir(managed.join(".git/objects/pack"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    if let Some(pack) = packs.iter().find(|path| {
        path.extension()
            .is_some_and(|extension| extension == "pack")
    }) {
        let bytes = std::fs::read(pack).unwrap();
        for path in packs {
            std::fs::remove_file(path).unwrap();
        }
        let mut unpack = f
            .command("git")
            .current_dir(&managed)
            .arg("unpack-objects")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        unpack.stdin.take().unwrap().write_all(&bytes).unwrap();
        success(unpack.wait_with_output().unwrap());
    }
    std::fs::remove_file(
        managed
            .join(".git/objects")
            .join(&blob[..2])
            .join(&blob[2..]),
    )
    .unwrap();
    success(f.cli().arg("check-release").output().unwrap());
    assert_eq!(
        f.git(&managed, &["show", &format!("{pin}:tracked.txt")]),
        "initial"
    );
}

struct DumbHttp {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DumbHttp {
    fn start(root: &Path) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let root = root.to_path_buf();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else {
                    break;
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let path = path.strip_prefix("repo/").unwrap_or(path);
                let body = if path.split('/').any(|component| component == "..") {
                    None
                } else {
                    std::fs::read(root.join(path)).ok()
                };
                let (status, body) =
                    body.map_or(("404 Not Found", Vec::new()), |body| ("200 OK", body));
                let _ = write!(
                    stream,
                    "HTTP/1.0 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        Self {
            url: format!("http://{address}/repo"),
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for DumbHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(
            self.url
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap(),
        );
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
