"""Run the generated native installer against real archives on loopback only."""

import functools
import http.server
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import tomllib

artifacts = Path("target/distrib").resolve()
target = sys.argv[1]
version = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
windows = os.name == "nt"
archive = f"sources-{target}" + (".zip" if windows else ".tar.gz")
if not (artifacts / archive).is_file():
    raise SystemExit(f"Missing native archive: {archive}")
script = artifacts / ("sources-installer.ps1" if windows else "sources-installer.sh")
command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)] if windows else ["sh", str(script)]
requests = []


class Handler(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        requests.append(self.path)
        super().do_GET()

    def log_message(self, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(artifacts)))
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
base = f"http://127.0.0.1:{server.server_port}"
try:
    # Exercise both documented download overrides and their failure paths. The
    # generated installer chooses the platform; the matrix target is an assertion.
    for override in ["SOURCES_DOWNLOAD_URL", "INSTALLER_DOWNLOAD_URL"]:
        for success in [True, False]:
            with tempfile.TemporaryDirectory(prefix="sources-installer-") as directory:
                root = Path(directory)
                prefix = root / ".local/bin"
                env = {key: value for key, value in os.environ.items()
                       if not key.startswith(("SOURCES_", "INSTALLER_", "CARGO_DIST_"))}
                env.update({override: base if success else base + "/missing",
                            "SOURCES_UNMANAGED_INSTALL": str(prefix),
                            "SOURCES_NO_MODIFY_PATH": "1"})
                preserved = {}
                if not windows:
                    # Exercise existing generated scripts, including references
                    # from a custom profile outside cargo-dist's migration list.
                    prefix.mkdir(parents=True)
                    preserved = {
                        prefix / "env": '''#!/bin/sh
# add binaries to PATH if they aren't added yet
# affix colons on either side of $PATH to simplify matching
case ":${PATH}:" in
    *:"$HOME/.local/bin":*)
        ;;
    *)
        # Prepending path in case a system-installed binary needs to be overridden
        export PATH="$HOME/.local/bin:$PATH"
        ;;
esac
''',
                        prefix / "env.fish": '''if not contains "$HOME/.local/bin" $PATH
    # Prepending path in case a system-installed binary needs to be overridden
    set -x PATH "$HOME/.local/bin" $PATH
end
''',
                        root / ".profile": '. "$HOME/.local/bin/env"\n',
                        root / ".zpath": '. "$HOME/.local/bin/env"\n',
                        root / ".config/fish/conf.d/sources.env.fish":
                            'source "$HOME/.local/bin/env.fish"\n',
                    }
                    for path, contents in preserved.items():
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_text(contents, encoding="utf-8")
                    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / ".config"),
                               ZDOTDIR=str(root), GITHUB_PATH=str(root / "github-path"))
                    # Unmanaged installs alone must also imply no path changes.
                    if override == "INSTALLER_DOWNLOAD_URL":
                        env.pop("SOURCES_NO_MODIFY_PATH")
                before = len(requests)
                result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90)
                output = result.stdout + result.stderr
                for path, contents in preserved.items():
                    if not path.is_file() or path.read_text(encoding="utf-8") != contents:
                        raise SystemExit(f"Installer modified existing shell configuration: {path}\n{output}")
                if not windows:
                    for path in [root / ".config/sources/env.sh", root / ".config/sources/env.fish",
                                 root / "github-path"]:
                        if path.exists():
                            raise SystemExit(f"Installer changed PATH configuration: {path}\n{output}")
                binary = prefix / ("sources.exe" if windows else "sources")
                instruction = "npx skills add refinio/sources"
                if success:
                    if result.returncode != 0 or instruction not in output:
                        raise SystemExit(f"{override} successful install failed ({result.returncode}):\n{output}")
                    if f"/{archive}" not in requests[before:]:
                        raise SystemExit(f"Installer did not select native archive {archive}: {requests[before:]}")
                    installed = subprocess.check_output([binary, "--version"], text=True, timeout=10).strip()
                    if installed != f"sources {version}":
                        raise SystemExit(f"Unexpected installed binary version: {installed}")
                elif result.returncode == 0 or instruction in output or binary.exists():
                    raise SystemExit(f"{override} failed download reported success ({result.returncode}):\n{output}")
                print(f"{target}: {override} {'success' if success else 'download failure'} passed", flush=True)
finally:
    server.shutdown()
    server.server_close()
    thread.join()
