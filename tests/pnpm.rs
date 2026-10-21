mod common;

use common::{Fixture, failure, success};
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Registry {
    child: Child,
    url: String,
}

impl Registry {
    fn start(f: &Fixture) -> Self {
        let mut child = f
            .command("python3")
            .args(["-u", "-c", REGISTRY])
            .arg(&f.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut port = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut port)
            .unwrap();
        let port: u16 = port.trim().parse().expect("fixture registry must start");
        Self {
            child,
            url: format!("http://127.0.0.1:{port}/"),
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct NpmFixture {
    f: Fixture,
    _registry: Registry,
    dev: PathBuf,
}

impl NpmFixture {
    fn new() -> Self {
        let f = Fixture::new();
        let registry = Registry::start(&f);
        let upstream = f.repo("upstream");
        for (name, content) in [
            ("a", "'local-a/' + require('@sources-probe/b')"),
            ("b", "'local-b'"),
        ] {
            let package = upstream.join("packages").join(name);
            std::fs::create_dir_all(&package).unwrap();
            let mut manifest = json!({"name":format!("@sources-probe/{name}"),"version":"1.0.0","main":"index.js"});
            if name == "a" {
                manifest["dependencies"] = json!({"@sources-probe/b":"workspace:*"});
            }
            write_json(&package.join("package.json"), &manifest);
            std::fs::write(
                package.join("index.js"),
                format!("module.exports = {content};\n"),
            )
            .unwrap();
        }
        f.git(&upstream, &["add", "."]);
        f.git(&upstream, &["commit", "-qm", "npm packages"]);
        let dev = f.root.join("dev");
        f.git(
            &f.root,
            &["clone", upstream.to_str().unwrap(), dev.to_str().unwrap()],
        );
        f.configure(
            json!({"sdk":{"url":upstream,"pnpm":{
                "@sources-probe/a":{"path":"packages/a"},
                "@sources-probe/b":{"path":"packages/b"}
            }}}),
            json!({"sdk":dev}),
        );
        write_json(
            &f.app.join("package.json"),
            &json!({"name":"consumer","private":true,"dependencies":{"@sources-probe/a":"^1.0.0"}}),
        );
        std::fs::write(f.app.join("pnpm-workspace.yaml"), "# keep this workspace comment\npackages:\n  - apps/*\noverrides:\n  unrelated-package: 2.3.4 # keep unrelated override\n").unwrap();
        std::fs::write(
            f.app.join(".npmrc"),
            format!(
                "registry={}\nstore-dir={}\nfetch-retries=0\nupdate-notifier=false\n",
                registry.url,
                f.root.join("pnpm-store").display()
            ),
        )
        .unwrap();
        std::fs::write(f.root.join("empty-npmrc"), "").unwrap();
        success(
            f.cli()
                .args([
                    "lock",
                    "--registry",
                    "@sources-probe/a:1.0.0",
                    "@sources-probe/b:1.0.0",
                ])
                .output()
                .unwrap(),
        );
        Self {
            f,
            _registry: registry,
            dev,
        }
    }

    fn cli(&self) -> Command {
        let mut command = self.f.cli();
        for (key, _) in std::env::vars_os() {
            let key_upper = key.to_string_lossy().to_ascii_uppercase();
            if key_upper.starts_with("NPM_") || key_upper.starts_with("PNPM_") {
                command.env_remove(key);
            }
        }
        command
            .env("NPM_CONFIG_USERCONFIG", self.f.root.join("empty-npmrc"))
            .env("NPM_CONFIG_GLOBALCONFIG", self.f.root.join("empty-npmrc"))
            .env("XDG_DATA_HOME", self.f.root.join("home"))
            .env("XDG_CACHE_HOME", self.f.root.join("home"))
            .env("CI", "true");
        command
    }

    fn require(&self) -> String {
        let output = success(
            self.f
                .command("node")
                .current_dir(&self.f.app)
                .args(["-p", "require('@sources-probe/a')"])
                .output()
                .unwrap(),
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn dev_and_release_switch_real_installed_packages_and_keep_unrelated_yaml() {
    let n = NpmFixture::new();
    success(n.cli().arg("dev").output().unwrap());
    assert_eq!(n.require(), "local-a/local-b");
    let status = success(n.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "file");
    let dev_lock = std::fs::read(n.f.app.join("pnpm-lock.yaml")).unwrap();
    success(n.cli().args(["release", "--registry"]).output().unwrap());
    assert_eq!(n.require(), "registry-a/registry-b");
    let status = success(n.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "registry");
    assert_eq!(status["sources"]["sdk"]["release_kind"], "pnpm");
    assert!(status["sources"]["sdk"]["active_commit"].is_null());
    failure(
        n.cli()
            .args(["run", "sdk", "--", "node", "--version"])
            .output()
            .unwrap(),
        "sources dev",
    );
    assert_ne!(
        std::fs::read(n.f.app.join("pnpm-lock.yaml")).unwrap(),
        dev_lock
    );
    assert!(!n.f.app.join(".sources/checkouts/sdk").exists());
    success(n.cli().arg("dev").output().unwrap());
    assert_eq!(n.require(), "local-a/local-b");
    let yaml = std::fs::read_to_string(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    assert!(yaml.contains("# keep this workspace comment"));
    assert!(yaml.contains("unrelated-package: 2.3.4 # keep unrelated override"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(n.f.app.join("package.json")).unwrap()).unwrap();
    assert_eq!(manifest["dependencies"]["@sources-probe/a"], "^1.0.0");
}

#[test]
fn legacy_json_overrides_are_rejected_before_selection_files_change() {
    let n = NpmFixture::new();
    let path = n.f.app.join("package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["pnpm"] = json!({"overrides":{"unrelated-package":"2.0.0"}});
    write_json(&path, &manifest);
    let before = std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    failure(
        n.cli().arg("dev").output().unwrap(),
        "package.json pnpm.overrides",
    );
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap(),
        before
    );
    assert!(!n.f.app.join("sources/sdk").exists());
}

#[test]
fn registry_release_offline_uses_store_without_development_checkout_or_git_upstream() {
    let mut n = NpmFixture::new();
    std::fs::write(n.f.app.join("pnpm-workspace.yaml"), "packages: []\n").unwrap();
    success(n.cli().args(["release", "--registry"]).output().unwrap());
    assert_eq!(n.require(), "registry-a/registry-b");
    success(n.cli().arg("dev").output().unwrap());
    n._registry.child.kill().unwrap();
    n._registry.child.wait().unwrap();
    let requests = std::fs::read(n.f.root.join("registry-requests.log")).unwrap();
    std::fs::rename(&n.dev, n.f.root.join("absent-dev")).unwrap();
    std::fs::rename(n.f.root.join("upstream"), n.f.root.join("absent-upstream")).unwrap();
    std::fs::remove_dir_all(n.f.app.join("node_modules")).unwrap();
    success(
        n.cli()
            .args(["release", "--registry", "--offline"])
            .output()
            .unwrap(),
    );
    assert_eq!(n.require(), "registry-a/registry-b");
    assert_eq!(
        std::fs::read(n.f.root.join("registry-requests.log")).unwrap(),
        requests
    );
    assert!(!n.f.app.join(".sources/checkouts/sdk").exists());
    assert!(n.f.app.join(".sources.lock").exists());
}

#[test]
fn failed_registry_install_reports_repair_and_rerunning_release_fixes_it() {
    let n = NpmFixture::new();
    success(n.cli().arg("dev").output().unwrap());
    let config_path = n.f.app.join(".sources.lock");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["sources"]["sdk"]["pnpm"]["@sources-probe/a"] = json!("2.0.0");
    write_json(&config_path, &config);
    let output = n.cli().args(["release", "--registry"]).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("ERR_PNPM_NO_MATCHING_VERSION"));
    failure(output, "rerun sources release");
    let yaml = std::fs::read_to_string(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    assert!(
        yaml.contains("2.0.0"),
        "a failed install need not undo selection changes"
    );
    config["sources"]["sdk"]["pnpm"]["@sources-probe/a"] = json!("1.0.0");
    write_json(&config_path, &config);
    success(n.cli().args(["release", "--registry"]).output().unwrap());
    assert_eq!(n.require(), "registry-a/registry-b");
}

#[test]
fn scoped_overrides_that_defeat_managed_selection_are_rejected_without_writes() {
    for selector in ["parent>@sources-probe/a", "@sources-probe/a@^1"] {
        let n = NpmFixture::new();
        let path = n.f.app.join("pnpm-workspace.yaml");
        let contents = format!("packages: []\noverrides:\n  '{selector}': 9.0.0\n");
        std::fs::write(&path, &contents).unwrap();
        failure(
            n.cli().arg("dev").output().unwrap(),
            "conflicts with managed package",
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
        assert!(!n.f.app.join("sources/sdk").exists());
    }
}

#[test]
fn mapped_packages_cannot_also_be_pnpm_workspace_members() {
    let n = NpmFixture::new();
    let path = n.f.app.join("pnpm-workspace.yaml");
    std::fs::write(&path, "packages:\n  - sources/*/packages/*\n").unwrap();
    failure(
        n.cli().arg("dev").output().unwrap(),
        "also a workspace member",
    );
    assert!(
        !std::fs::read_to_string(path)
            .unwrap()
            .contains("overrides:")
    );
}

#[test]
fn invalid_local_package_name_fails_before_workspace_or_links_change() {
    let n = NpmFixture::new();
    let package = n.dev.join("packages/a/package.json");
    write_json(
        &package,
        &json!({"name":"different-package","version":"1.0.0"}),
    );
    let before = std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    failure(
        n.cli().arg("dev").output().unwrap(),
        "must declare package name @sources-probe/a",
    );
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap(),
        before
    );
    assert!(!n.f.app.join("sources/sdk").exists());
}

#[test]
fn mixed_git_and_registry_sources_lock_and_activate_their_own_release_inputs() {
    let n = NpmFixture::new();
    let upstream = n.f.repo("plain-upstream");
    let dev = n.f.root.join("plain-dev");
    n.f.git(
        &n.f.root,
        &["clone", upstream.to_str().unwrap(), dev.to_str().unwrap()],
    );
    let config_path = n.f.app.join(".sources.conf");
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).unwrap()).unwrap();
    config["sources"]["plain"] = json!({"url":upstream});
    write_json(&config_path, &config);
    let local_path = n.f.app.join(".sources.local.conf");
    let mut local: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&local_path).unwrap()).unwrap();
    local["sources"]["plain"] = json!(dev);
    write_json(&local_path, &local);
    success(n.cli().arg("lock").output().unwrap());
    let lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(n.f.app.join(".sources.lock")).unwrap()).unwrap();
    assert_eq!(
        lock["sources"]["plain"]["git"],
        n.f.git(&dev, &["rev-parse", "HEAD"])
    );
    assert!(lock["sources"]["sdk"]["git"].is_string());
    let absent = n.f.root.join("absent-plain-upstream");
    std::fs::rename(&upstream, &absent).unwrap();
    failure(
        n.cli().args(["release", "--registry"]).output().unwrap(),
        "rerun sources release --registry",
    );
    std::fs::rename(&absent, &upstream).unwrap();
    success(n.cli().args(["release", "--registry"]).output().unwrap());
    assert_eq!(n.require(), "registry-a/registry-b");
    assert!(
        n.f.app
            .join(".sources/checkouts/plain/tracked.txt")
            .is_file()
    );
    assert!(!n.f.app.join(".sources/checkouts/sdk").exists());
    success(
        n.cli()
            .args(["run", "plain", "--", "git", "rev-parse", "HEAD"])
            .output()
            .unwrap(),
    );
    let status = success(n.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["mode"], "release");
    assert_eq!(status["sources"]["plain"]["release_kind"], "git");
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "registry");
}

#[test]
fn invalid_owned_override_values_are_reported_as_mixed_selection() {
    let n = NpmFixture::new();
    std::fs::write(
        n.f.app.join("pnpm-workspace.yaml"),
        "packages: []\noverrides:\n  '@sources-probe/a': {}\n  '@sources-probe/b': []\n",
    )
    .unwrap();
    let output = n.cli().args(["status", "--json"]).output().unwrap();
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "mixed");
    failure(output, "pnpm references are inconsistent");
}

fn replace_registry_dev_link_with_managed_checkout(n: &NpmFixture) -> PathBuf {
    let managed = n.f.app.join(".sources/checkouts/sdk");
    std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
    n.f.git(
        &n.f.root,
        &[
            "clone",
            n.f.root.join("upstream").to_str().unwrap(),
            managed.to_str().unwrap(),
        ],
    );
    let link = n.f.app.join("sources/sdk");
    #[cfg(unix)]
    {
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&managed, link).unwrap();
    }
    #[cfg(windows)]
    {
        std::fs::remove_dir(&link).unwrap();
        std::os::windows::fs::symlink_dir(&managed, link).unwrap();
    }
    managed
}

#[test]
fn file_selection_run_accepts_a_valid_managed_git_checkout() {
    let n = NpmFixture::new();
    success(n.cli().arg("dev").output().unwrap());
    assert_eq!(n.require(), "local-a/local-b");
    let managed = replace_registry_dev_link_with_managed_checkout(&n);
    success(
        n.cli()
            .args([
                "run",
                "sdk",
                "--",
                "node",
                "-e",
                "require('node:fs').writeFileSync('wrong-checkout-run', 'ran')",
            ])
            .output()
            .unwrap(),
    );
    assert!(managed.join("wrong-checkout-run").exists());
}

#[test]
fn registry_status_rejects_managed_git_link_with_unconfigured_package_selection() {
    let n = NpmFixture::new();
    success(n.cli().arg("dev").output().unwrap());
    replace_registry_dev_link_with_managed_checkout(&n);
    std::fs::write(
        n.f.app.join("pnpm-workspace.yaml"),
        "packages: []\noverrides:\n  unrelated-package: 2.3.4\n",
    )
    .unwrap();
    let output = n.cli().args(["status", "--json"]).output().unwrap();
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "unconfigured");
    assert_eq!(status["sources"]["sdk"]["active_mode"], "mixed");
    failure(output, "links and pnpm references disagree");
}

#[cfg(unix)]
mod signals {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    struct Install {
        child: Option<Child>,
        pid: i32,
        group: i32,
    }

    impl Drop for Install {
        fn drop(&mut self) {
            unsafe {
                if self.group > 0 {
                    libc::kill(-self.group, libc::SIGKILL);
                }
                if self.pid > 0 {
                    libc::kill(self.pid, libc::SIGKILL);
                }
            }
            if let Some(child) = &mut self.child {
                let _ = child.wait();
            }
        }
    }

    fn readable(fd: RawFd) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let mut descriptor = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "install hook handshake timed out");
            let result = unsafe {
                libc::poll(
                    &mut descriptor,
                    1,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )
            };
            if result == -1
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            assert!(result > 0, "install hook did not reach its next marker");
            return;
        }
    }

    fn marker(stream: &mut TcpStream) -> u8 {
        readable(stream.as_raw_fd());
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        byte[0]
    }

    #[test]
    fn pnpm_install_keeps_lock_until_signaled_lifecycle_descendant_finishes() {
        let n = NpmFixture::new();
        let path = n.f.app.join("package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["scripts"] = json!({"postinstall":"node install-hook.cjs"});
        write_json(&path, &manifest);
        std::fs::write(n.f.app.join("install-hook.cjs"), r#"
const socket = require('node:net').connect(Number(process.env.SOURCES_PNPM_TEST_PORT), '127.0.0.1', () => {
  process.on('SIGTERM', () => socket.write('S'));
  socket.on('data', () => process.exit(17));
  const ready = Buffer.alloc(5);
  ready[0] = 'R'.charCodeAt(0);
  ready.writeUInt32BE(process.pid, 1);
  socket.write(ready);
});
"#).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut command = n.cli();
        command
            .arg("dev")
            .env(
                "SOURCES_PNPM_TEST_PORT",
                listener.local_addr().unwrap().port().to_string(),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let pid = child.id() as i32;
        let mut install = Install {
            child: Some(child),
            pid,
            group: pid,
        };
        readable(listener.as_raw_fd());
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_nonblocking(false).unwrap();
        assert_eq!(marker(&mut stream), b'R');
        let hook_pid = u32::from_be_bytes(std::array::from_fn(|_| marker(&mut stream))) as i32;
        let group = unsafe { libc::getpgid(hook_pid) };
        assert!(group > 0);
        install.group = group;
        assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
        assert_eq!(marker(&mut stream), b'S');
        failure(
            n.cli().args(["status", "--json"]).output().unwrap(),
            "locked",
        );
        assert!(
            install
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        stream.write_all(b"G").unwrap();
        let child = install.child.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        let output = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("Sources did not finish after the install hook exited")
            .unwrap();
        waiter.join().unwrap();
        install.pid = 0;
        install.group = 0;
        failure(output, "pnpm install failed");
        success(n.cli().args(["status", "--json"]).output().unwrap());
    }
}

const REGISTRY: &str = r#"
import base64, gzip, hashlib, http.server, io, json, pathlib, sys, tarfile, urllib.parse
root = pathlib.Path(sys.argv[1])
archives = {}
for name in ['a', 'b']:
    package = {'name':'@sources-probe/'+name,'version':'1.0.0','main':'index.js'}
    if name == 'a': package['dependencies'] = {'@sources-probe/b':'1.0.0'}
    content = "'registry-a/' + require('@sources-probe/b')" if name == 'a' else "'registry-b'"
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode='w:gz') as tar:
        for path, data in [('package/package.json', json.dumps(package).encode()), ('package/index.js', ('module.exports='+content+';\n').encode())]:
            info = tarfile.TarInfo(path); info.size=len(data); info.mtime=1704067200
            tar.addfile(info,io.BytesIO(data))
    archives[name] = (output.getvalue(), package)
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = urllib.parse.unquote(self.path).lstrip('/')
        with (root/'registry-requests.log').open('a') as log: log.write(path+'\n')
        if path in ['a.tgz','b.tgz']:
            if (root/'deny-tarballs').exists(): self.send_error(404);return
            data = archives[path[0]][0]
            self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.end_headers();self.wfile.write(data);return
        if path in ['@sources-probe/a','@sources-probe/b']:
            name=path[-1];data,package=archives[name]
            version=dict(package,dist={'tarball':f'http://127.0.0.1:{self.server.server_port}/{name}.tgz','integrity':'sha512-'+base64.b64encode(hashlib.sha512(data).digest()).decode(),'shasum':hashlib.sha1(data).hexdigest()})
            data=json.dumps({'name':path,'dist-tags':{'latest':'1.0.0'},'versions':{'1.0.0':version}}).encode()
            self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(data);return
        self.send_error(404)
    def log_message(self,*args): pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
print(server.server_port,flush=True)
server.serve_forever()
"#;

#[test]
fn registry_lock_check_downloads_without_switching_and_failure_preserves_pins() {
    let n = NpmFixture::new();
    let original = std::fs::read(n.f.app.join(".sources.lock")).unwrap();
    std::fs::write(n.f.root.join("deny-tarballs"), "").unwrap();
    let failed = n
        .cli()
        .args(["lock", "--registry", "--check", "@sources-probe/a:1.0.0"])
        .output()
        .unwrap();
    failure(failed, "registry download failed");
    assert_eq!(
        std::fs::read(n.f.app.join(".sources.lock")).unwrap(),
        original
    );
    std::fs::remove_file(n.f.root.join("deny-tarballs")).unwrap();
    let lock = n.f.app.join(".sources.lock");
    let before = std::fs::read(&lock).unwrap();
    let yaml = std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    success(
        n.cli()
            .args([
                "lock",
                "--registry",
                "--check",
                "@sources-probe/a:1.0.0",
                "@sources-probe/b:1.0.0",
            ])
            .output()
            .unwrap(),
    );
    let requests = std::fs::read_to_string(n.f.root.join("registry-requests.log")).unwrap();
    assert!(
        requests.contains(".tgz"),
        "must download the release archive: {requests}"
    );
    assert_eq!(std::fs::read(&lock).unwrap(), before);
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap(),
        yaml
    );
    assert!(!n.f.app.join("pnpm-lock.yaml").exists());
    assert!(!n.f.app.join("node_modules").exists());
    failure(
        n.cli()
            .args(["lock", "--registry", "--check", "@sources-probe/a:2.0.0"])
            .output()
            .unwrap(),
        "registry download failed",
    );
    assert_eq!(std::fs::read(&lock).unwrap(), before);
}

#[test]
fn local_and_pinned_git_share_file_references_and_leave_dependency_lock_unchanged() {
    let n = NpmFixture::new();
    success(n.cli().args(["lock", "--git", "--check"]).output().unwrap());
    success(n.cli().arg("dev").output().unwrap());
    let lock = std::fs::read(n.f.app.join(".sources.lock")).unwrap();
    let pnpm_lock = std::fs::read(n.f.app.join("pnpm-lock.yaml")).unwrap();
    let yaml = std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    std::fs::write(
        n.dev.join("packages/b/index.js"),
        "module.exports = 'edited-b';\n",
    )
    .unwrap();
    success(n.cli().args(["release", "--git"]).output().unwrap());
    let output = success(
        n.cli()
            .args(["run", "sdk", "--", "git", "rev-parse", "HEAD"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        n.f.git(&n.f.root.join("upstream"), &["rev-parse", "HEAD"])
    );
    let status = success(n.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["active_mode"], "release");
    assert_eq!(status["sources"]["sdk"]["pnpm_selection"], "file");
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-lock.yaml")).unwrap(),
        pnpm_lock
    );
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-workspace.yaml")).unwrap(),
        yaml
    );
    success(n.cli().arg("dev").output().unwrap());
    assert_eq!(
        std::fs::read(n.f.app.join("pnpm-lock.yaml")).unwrap(),
        pnpm_lock
    );
    assert_eq!(std::fs::read(n.f.app.join(".sources.lock")).unwrap(), lock);
    success(n.cli().args(["release", "--registry"]).output().unwrap());
    assert_eq!(n.require(), "registry-a/registry-b");
    let status = success(n.cli().args(["status", "--json"]).output().unwrap());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["sources"]["sdk"]["cached_commit_available"], true);
    success(n.cli().args(["release", "--git"]).output().unwrap());
    assert_eq!(n.require(), "local-a/local-b");
    assert_eq!(std::fs::read(n.f.app.join(".sources.lock")).unwrap(), lock);
}

#[test]
fn failed_file_install_can_be_retried_with_the_suggested_pnpm_command() {
    let n = NpmFixture::new();
    let path = n.f.app.join("package.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["scripts"] = json!({"postinstall":"node -e \"process.exit(7)\""});
    write_json(&path, &manifest);
    failure(
        n.cli().arg("dev").output().unwrap(),
        "rerun pnpm install --no-frozen-lockfile",
    );
    assert!(n.f.app.join("pnpm-lock.yaml").exists());
    manifest["scripts"] = json!({"postinstall":"node -e \"require('node:fs').writeFileSync('install-retried', 'yes')\""});
    write_json(&path, &manifest);
    success(
        n.f.command("pnpm")
            .current_dir(&n.f.app)
            .env("CI", "true")
            .env("NPM_CONFIG_USERCONFIG", n.f.root.join("empty-npmrc"))
            .env("NPM_CONFIG_GLOBALCONFIG", n.f.root.join("empty-npmrc"))
            .args(["install", "--no-frozen-lockfile"])
            .output()
            .unwrap(),
    );
    assert!(n.f.app.join("install-retried").exists());
    success(n.cli().arg("dev").output().unwrap());
    assert_eq!(n.require(), "local-a/local-b");
}
