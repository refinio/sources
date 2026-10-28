#![cfg(unix)]

mod common;

use common::{Fixture, success};
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};

struct Releases {
    child: Child,
    url: String,
}

impl Releases {
    fn start(f: &Fixture) -> Self {
        let mut child = f
            .command("python3")
            .args([
                "-u",
                "-c",
                "import http.server, os, sys\nos.chdir(sys.argv[1])\ns = http.server.ThreadingHTTPServer(('127.0.0.1', 0), http.server.SimpleHTTPRequestHandler)\nprint(s.server_port, flush=True)\ns.serve_forever()",
            ])
            .arg(&f.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        let port: u16 = port.trim().parse().expect("release server must start");
        Self {
            child,
            url: format!("http://127.0.0.1:{port}"),
        }
    }
}

impl Drop for Releases {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn update_replaces_the_running_binary_and_suggests_skills_only_after_success() {
    let f = Fixture::new();
    let server = Releases::start(&f);
    let bin = f.root.join("custom install/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let installed = bin.join("sources");
    std::fs::copy(assert_cmd::cargo::cargo_bin!("sources"), &installed).unwrap();
    let original = std::fs::read(&installed).unwrap();
    let update = |terminal: bool| {
        let mut command = if terminal {
            let mut command = f.command("python3");
            command
                .args([
                    "-c",
                    "import os, pty, subprocess, sys\nmaster, slave = pty.openpty()\ntry:\n result = subprocess.run(sys.argv[1:], stdin=slave, timeout=15)\nfinally:\n os.close(slave)\n os.close(master)\nsys.exit(result.returncode)",
                ])
                .arg(&installed);
            command
        } else {
            f.command(&installed)
        };
        command
            .current_dir(&f.app)
            .arg("update")
            .env("SOURCES_UPDATE_URL", format!("{}/installer.sh", server.url))
            // The running executable, rather than Cargo's default or inherited
            // installer settings, determines which installation gets updated.
            .env("CARGO_HOME", f.root.join("other-cargo"))
            .env("SOURCES_UNMANAGED_INSTALL", f.root.join("other-install"));
        command
    };

    let failed = update(false).output().unwrap();
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert!(!String::from_utf8_lossy(&failed.stdout).contains("npx skills update"));
    assert_eq!(std::fs::read(&installed).unwrap(), original);

    std::fs::write(
        f.root.join("installer.sh"),
        "#!/bin/sh\nif [ \"$SOURCES_PRINT_QUIET\" = 0 ]; then echo 'installer prerequisite missing: tar' >&2; fi\nexit 23\n",
    )
    .unwrap();
    let failed = update(false).output().unwrap();
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("installer prerequisite missing: tar")
    );
    assert!(!String::from_utf8_lossy(&failed.stdout).contains("npx skills update"));
    assert_eq!(std::fs::read(&installed).unwrap(), original);

    std::fs::write(
        f.root.join("installer.sh"),
        format!(
            "#!/bin/sh\nset -eu\nif [ -t 0 ]; then echo 'installer would prompt' >&2; exit 24; fi\ncurl -fsSL '{}/sources' -o \"$SOURCES_UNMANAGED_INSTALL/sources.new\"\nchmod +x \"$SOURCES_UNMANAGED_INSTALL/sources.new\"\nmv -f \"$SOURCES_UNMANAGED_INSTALL/sources.new\" \"$SOURCES_UNMANAGED_INSTALL/sources\"\n",
            server.url
        ),
    )
    .unwrap();
    std::fs::write(
        f.root.join("sources"),
        "#!/bin/sh\nprintf 'sources 99.0.0\\n'\n",
    )
    .unwrap();
    let output = success(update(true).output().unwrap());
    assert!(String::from_utf8_lossy(&output.stdout).contains("npx skills update"));
    let version = success(f.command(&installed).arg("--version").output().unwrap());
    assert_eq!(String::from_utf8_lossy(&version.stdout), "sources 99.0.0\n");
    assert!(!f.app.join(".sources").exists());
    assert!(!f.root.join("other-install").exists());
    assert!(!f.root.join("other-cargo").exists());
}
