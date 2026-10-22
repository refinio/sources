mod common;
#[path = "common/turbo.rs"]
mod turbo_fixture;

use common::success;
use turbo_fixture::TurboFixture;

#[test]
fn task_selectors_preserve_exact_pairs_and_allow_turbo_prerequisites() {
    let t = TurboFixture::new();
    success(
        t.cli()
            .args(["task", "@example/protocol:build", "@example/ui:pack"])
            .output()
            .unwrap(),
    );
    assert!(t.marker("protocol", "prepare").is_file());
    assert!(t.marker("protocol", "build").is_file());
    assert!(t.marker("ui", "pack").is_file());
    assert!(!t.marker("protocol", "pack").exists());
    assert!(!t.marker("ui", "build").exists());
    assert!(!t.marker("ui", "prepare").exists());
}
