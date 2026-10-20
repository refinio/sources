#!/usr/bin/env bash
# Build the acceptance-test Git version without changing the runner's installation.
set -euo pipefail
version="$1"
case "$version" in
  2.56.0) checksum=26c56c296b38c0695b26fa95f475f1d01704d2d38e73465ca30b0b2f5dc789d3 ;;
  2.39.5) checksum=c58da92c378df4a986ca33266897a7397e86c22ee266a284d8c2432c39066b59 ;;
  *) echo "Unsupported test Git version: $version" >&2; exit 1 ;;
esac
build_dir="${RUNNER_TEMP:?}/sources-git-$version"
prefix="$build_dir/install"
mkdir -p "$build_dir"
curl --fail --location --retry 3 \
  "https://www.kernel.org/pub/software/scm/git/git-$version.tar.xz" \
  --output "$build_dir/git.tar.xz"
python3 - "$build_dir/git.tar.xz" "$checksum" <<'PY'
import hashlib, pathlib, sys
archive = pathlib.Path(sys.argv[1])
if hashlib.sha256(archive.read_bytes()).hexdigest() != sys.argv[2]:
    sys.exit(f"Git archive checksum mismatch: {archive}")
PY
tar -xJf "$build_dir/git.tar.xz" -C "$build_dir"
# Git 2.39 predates Apple's CommonCrypto header changes. OpenSSL works on both macOS CPUs.
if [[ "$(uname)" == Darwin ]]; then
  openssl_prefix="$(brew --prefix openssl@3)"
  make -C "$build_dir/git-$version" -j2 prefix="$prefix" \
    CFLAGS="-O2 -std=gnu11" OPENSSLDIR="$openssl_prefix" NO_APPLE_COMMON_CRYPTO=YesPlease \
    NO_GETTEXT=YesPlease NO_TCLTK=YesPlease all install
else
  make -C "$build_dir/git-$version" -j2 prefix="$prefix" \
    CFLAGS="-O2 -std=gnu11" NO_GETTEXT=YesPlease NO_TCLTK=YesPlease all install
fi
"$prefix/bin/git" --version
printf '%s\n' "$prefix/bin" >> "$GITHUB_PATH"
