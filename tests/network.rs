mod common;
use common::{Fixture, failure, success};
use serde_json::json;

#[test]
fn loopback_fetch_is_shallow_and_cached_readiness_does_not_require_a_running_server() {
    let f = Fixture::new();
    let upstream = f.repo("upstream");
    std::fs::write(upstream.join("tracked.txt"), "selected revision\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "selected"]);
    let commit = f.git(&upstream, &["rev-parse", "HEAD"]);
    let server = f.server("upstream");
    f.configure(json!({"a":{"url":server.url}}), json!({}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 2, "sources": {"a": {"git": commit}}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["rev-list", "--count", &commit]), "1");
    assert_eq!(
        f.git(&managed, &["show", &format!("{commit}:tracked.txt")]),
        "selected revision"
    );
    assert_eq!(
        f.git(&managed, &["config", "--get", "remote.origin.url"]),
        server.url
    );
    drop(server);
    success(f.cli().arg("check-release").output().unwrap());
    failure(f.cli().arg("fetch").output().unwrap(), "fetch failed");
    assert!(!f.app.join("sources").exists());
}
