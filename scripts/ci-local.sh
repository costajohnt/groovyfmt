#!/usr/bin/env bash
# The checks CI runs on Linux, for running on your own machine before a merge.
# GitHub Actions only runs for releases (and when started by hand), where it also
# covers macOS and the release builds. Usage: scripts/ci-local.sh [corpus-dir]
set -euo pipefail
cd "$(dirname "$0")/.."
msrv=$(sed -n 's/^rust-version = "\(.*\)"$/\1/p' Cargo.toml)

cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo "+$msrv" test --locked
cargo build --release --locked

bin=target/release/groovyfmt
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf 'class A {\ndef f() {\nprintln 1\n}\n}\n' > "$tmp/A.groovy"
if "$bin" --check "$tmp/A.groovy" > /dev/null; then echo "check passed on an unformatted file" >&2; exit 1; fi
"$bin" "$tmp/A.groovy"
"$bin" --check "$tmp/A.groovy"

if command -v musl-gcc > /dev/null && rustup target list --installed | grep -qx x86_64-unknown-linux-musl; then
  CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --locked --target x86_64-unknown-linux-musl
  file target/x86_64-unknown-linux-musl/release/groovyfmt | grep -Eq 'statically linked|static-pie linked'
else
  echo "skipped: static musl build (needs musl-gcc and the x86_64-unknown-linux-musl target)"
fi

# Formatting a real corpus must stay idempotent; --stats exits 1 if any file is not.
if [ $# -gt 0 ]; then "$bin" --stats "$1" | tail -n 1; fi
echo "ci-local: ok"
