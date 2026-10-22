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
    let workspace = std::fs::read_to_string(n.f.app.join("pnpm-workspace.yaml")).unwrap();
    assert!(workspace.contains("# keep this workspace comment"));
    assert!(workspace.contains("unrelated-package: 2.3.4 # keep unrelated override"));
}
