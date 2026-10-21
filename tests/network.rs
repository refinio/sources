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

#[cfg(unix)]
#[test]
fn ssh_transport_caches_shallow_complete_pins_without_modifying_development_sources() {
    ssh_transport_fixture(true);
    ssh_transport_fixture(false);
}

#[cfg(unix)]
fn ssh_transport_fixture(native: bool) {
    let f = Fixture::new();
    let upstream = f.repo("upstream ssh");
    std::fs::write(upstream.join("tracked.txt"), "selected SSH contents\n").unwrap();
    f.git(&upstream, &["commit", "-qam", "SSH input"]);
    let pin = f.git(&upstream, &["rev-parse", "HEAD"]);
    let endpoint = SshEndpoint::start(&f, &upstream, native);
    let dev = f.repo("development");
    f.git(&dev, &["remote", "add", "origin", &endpoint.url]);
    f.git(
        &dev,
        &[
            "remote",
            "add",
            "credential-redaction",
            "https://fixture-user:fixture-secret@example.invalid/team/repo",
        ],
    );
    std::fs::write(dev.join("tracked.txt"), "development edit\n").unwrap();
    let dev_head = f.git(&dev, &["rev-parse", "HEAD"]);
    let dev_status = f.git(&dev, &["status", "--porcelain=v1"]);
    let dev_objects = f.git(&dev, &["count-objects", "-v"]);
    f.configure(json!({"a": {"url": endpoint.url}}), json!({"a": dev}));
    std::fs::write(
        f.app.join(".sources.lock"),
        json!({"version": 2, "sources": {"a": {"git": pin}}}).to_string(),
    )
    .unwrap();
    success(f.cli().arg("fetch").output().unwrap());
    let managed = f.app.join(".sources/checkouts/a");
    assert_eq!(f.git(&managed, &["rev-list", "--count", &pin]), "1");
    assert_eq!(
        f.git(&managed, &["show", &format!("{pin}:tracked.txt")]),
        "selected SSH contents"
    );
    assert_eq!(
        f.git(&managed, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    assert_eq!(f.git(&dev, &["rev-parse", "HEAD"]), dev_head);
    assert_eq!(f.git(&dev, &["status", "--porcelain=v1"]), dev_status);
    assert_eq!(f.git(&dev, &["count-objects", "-v"]), dev_objects);
    assert!(!dev.join(".git/FETCH_HEAD").exists());
    assert_eq!(
        std::fs::read_to_string(dev.join("tracked.txt")).unwrap(),
        "development edit\n"
    );
    assert!(!f.app.join("sources").exists());
    drop(endpoint);
    let adapter = f.root.join("ssh-transport");
    if adapter.exists() {
        std::fs::remove_file(adapter).unwrap();
    }
    success(f.cli().arg("check-release").output().unwrap());
    let transcript = std::fs::read_to_string(f.root.join("git-commands.jsonl")).unwrap();
    assert!(!transcript.contains("fixture-secret") && !transcript.contains("fixture-user"));
    assert!(transcript.contains("[redacted]@example.invalid"));
    for line in transcript.lines() {
        let entry: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(
            entry["arguments"].is_array() && entry["status"] == 0 && entry["stderr"].is_string()
        );
    }
}

#[cfg(unix)]
struct SshEndpoint {
    url: String,
    child: Option<std::process::Child>,
}

#[cfg(unix)]
impl SshEndpoint {
    fn start(f: &Fixture, upstream: &std::path::Path, native: bool) -> Self {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        use std::process::Stdio;
        use std::time::{Duration, Instant};
        let executable = |name: &str| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|directory| directory.join(name))
                .find(|path| path.is_file())
                .map(|path| path.canonicalize().unwrap())
        };
        let exec_path = f.git(&f.root, &["--exec-path"]);
        let upload_pack = std::path::Path::new(&exec_path).join("git-upload-pack");
        let gitconfig = f.root.join("gitconfig");
        f.git(
            &f.root,
            &[
                "config",
                "--file",
                gitconfig.to_str().unwrap(),
                "ssh.variant",
                "ssh",
            ],
        );
        if let (true, Some(sshd), Some(ssh), Some(keygen)) = (
            native,
            executable("sshd"),
            executable("ssh"),
            executable("ssh-keygen"),
        ) {
            for name in ["ssh-host", "ssh-client"] {
                success(
                    f.command(&keygen)
                        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                        .arg(f.root.join(name))
                        .output()
                        .unwrap(),
                );
            }
            std::fs::copy(
                f.root.join("ssh-client.pub"),
                f.root.join("authorized_keys"),
            )
            .unwrap();
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let username =
                String::from_utf8(success(f.command("id").arg("-un").output().unwrap()).stdout)
                    .unwrap();
            let config = f.root.join("sshd.conf");
            let text = format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {}/ssh-host\nPidFile {}/sshd.pid\nAuthorizedKeysFile {}/authorized_keys\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPermitRootLogin yes\nAllowUsers {}\nLogLevel ERROR\nSetEnv HOME={} ZDOTDIR={} GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL={} PATH={exec_path}:/usr/bin:/bin\n",
                f.root.display(),
                f.root.display(),
                f.root.display(),
                username.trim(),
                f.root.display(),
                f.root.display(),
                gitconfig.display()
            );
            std::fs::write(&config, text).unwrap();
            let host_key = std::fs::read_to_string(f.root.join("ssh-host.pub")).unwrap();
            std::fs::write(
                f.root.join("known_hosts"),
                format!("[127.0.0.1]:{port} {host_key}"),
            )
            .unwrap();
            let client = f.root.join("ssh-client.conf");
            std::fs::write(&client, format!("Host *\nBatchMode yes\nIdentitiesOnly yes\nIdentityFile {}/ssh-client\nUserKnownHostsFile {}/known_hosts\nStrictHostKeyChecking yes\nConnectTimeout 5\nLogLevel ERROR\n", f.root.display(), f.root.display())).unwrap();
            f.git(
                &f.root,
                &[
                    "config",
                    "--file",
                    gitconfig.to_str().unwrap(),
                    "core.sshCommand",
                    &format!(
                        "{} -F {}",
                        quote(ssh.to_str().unwrap()),
                        quote(client.to_str().unwrap())
                    ),
                ],
            );
            let log = std::fs::File::create(f.root.join("sshd.log")).unwrap();
            let mut url = url::Url::from_file_path(upstream).unwrap();
            let path = url.path().to_owned();
            url = url::Url::parse(&format!("ssh://{}@127.0.0.1:{port}", username.trim())).unwrap();
            url.set_path(&path);
            let child = f
                .command(sshd)
                .args(["-D", "-e", "-f"])
                .arg(config)
                .stdout(Stdio::null())
                .stderr(log)
                .spawn();
            match child {
                Ok(child) => {
                    let mut endpoint = Self {
                        url: url.to_string(),
                        child: Some(child),
                    };
                    let deadline = Instant::now() + Duration::from_secs(5);
                    let unavailable = loop {
                        match endpoint.child.as_mut().unwrap().try_wait() {
                            Ok(Some(status)) => {
                                break Some(format!(
                                    "exited {status}: {}",
                                    std::fs::read_to_string(f.root.join("sshd.log"))
                                        .unwrap_or_default()
                                ));
                            }
                            Err(error) => break Some(error.to_string()),
                            Ok(None) => {}
                        }
                        if TcpStream::connect_timeout(
                            &format!("127.0.0.1:{port}").parse().unwrap(),
                            Duration::from_millis(20),
                        )
                        .is_ok()
                        {
                            break None;
                        }
                        if Instant::now() >= deadline {
                            break Some("daemon did not begin listening".to_owned());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    };
                    let unavailable = unavailable.or_else(|| {
                        let probe = f
                            .command(&ssh)
                            .arg("-F")
                            .arg(&client)
                            .args(["-p", &port.to_string()])
                            .arg(format!("{}@127.0.0.1", username.trim()))
                            .arg("printf sources-ssh-ready")
                            .stdin(Stdio::null())
                            .output();
                        match probe {
                            Ok(output)
                                if output.status.success()
                                    && output.stdout == b"sources-ssh-ready" =>
                            {
                                None
                            }
                            Ok(output) => Some(format!(
                                "authenticated remote-command probe failed ({}): {}{}",
                                output.status,
                                String::from_utf8_lossy(&output.stderr),
                                String::from_utf8_lossy(&output.stdout)
                            )),
                            Err(error) => {
                                Some(format!("cannot run authenticated SSH probe: {error}"))
                            }
                        }
                    });
                    match unavailable {
                        None => return endpoint,
                        Some(error) => eprintln!(
                            "fixture sshd unavailable: {}; using local Git SSH adapter",
                            sources::git::redact(error.trim())
                        ),
                    }
                }
                Err(error) => {
                    eprintln!("fixture sshd unavailable: {error}; using local Git SSH adapter")
                }
            }
        }
        // Exercise Git's SSH command boundary without requiring a system SSH service.
        let script = f.root.join("ssh-transport");
        let mut file = std::fs::File::create(&script).unwrap();
        writeln!(file, "#!/bin/sh\nfor argument do remote=$argument; done\n[ \"$remote\" = {} ] || exit 93\nexec {} {}", quote(&format!("git-upload-pack {}", quote(upstream.to_str().unwrap()))), quote(upload_pack.to_str().unwrap()), quote(upstream.to_str().unwrap())).unwrap();
        f.git(
            &f.root,
            &[
                "config",
                "--file",
                gitconfig.to_str().unwrap(),
                "core.sshCommand",
                &format!("sh {}", quote(script.to_str().unwrap())),
            ],
        );
        let mut url = url::Url::parse("ssh://fixture@ssh.invalid").unwrap();
        url.set_path(upstream.to_str().unwrap());
        Self {
            url: url.to_string(),
            child: None,
        }
    }
}

#[cfg(unix)]
impl Drop for SshEndpoint {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
