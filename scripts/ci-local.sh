#!/usr/bin/env bash
# The checks CI runs on Linux, for running on your own machine before a merge.
# GitHub Actions runs the same checks on pull requests, pushes to main and
# releases, where it also covers macOS, the release builds and the install
# channels (the macOS builds only for a release or a manual run).
# Usage: scripts/ci-local.sh [corpus-dir]
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

# Windows: cross-compiled with mingw-w64, every test run under Wine (scripts/wine-run.sh).
win=x86_64-pc-windows-gnu
if command -v x86_64-w64-mingw32-gcc > /dev/null && rustup target list --installed | grep -qx "$win" \
  && { command -v wine > /dev/null || command -v wine64 > /dev/null || [ -x /usr/lib/wine/wine64 ]; }; then
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=$PWD/scripts/wine-run.sh
  export WINEPREFIX=${WINEPREFIX:-$HOME/.wine-gf}
  wineserver=$(command -v wineserver || echo /usr/lib/wine/wineserver64)
  trap 'rm -rf "$tmp"; "$wineserver" -k 2> /dev/null || true' EXIT
  cargo clippy --locked --all-targets --target "$win" -- -D warnings
  if rustup "+$msrv" target list --installed | grep -qx "$win"; then
    cargo "+$msrv" check --locked --all-targets --target "$win"
  fi
  cargo test --locked --target "$win"
  cargo build --release --locked --target "$win"
  exe=target/$win/release/groovyfmt.exe
  # only Windows' own DLLs, no mingw runtime
  if x86_64-w64-mingw32-objdump -p "$exe" | grep 'DLL Name' \
    | grep -viE 'DLL Name: (kernel32|ntdll|msvcrt|ws2_32|userenv|advapi32|bcrypt|bcryptprimitives|api-ms-win-[a-z0-9-]+)\.dll$'; then
    echo "$exe imports a DLL that isn't part of Windows" >&2
    exit 1
  fi
  printf 'class A {\ndef f() {\nprintln 1\n}\n}\n' > "$tmp/W.groovy"
  if scripts/wine-run.sh "$exe" --check "$tmp/W.groovy" > /dev/null; then echo "check passed on an unformatted file under Wine" >&2; exit 1; fi
  scripts/wine-run.sh "$exe" "$tmp/W.groovy"
  scripts/wine-run.sh "$exe" --check "$tmp/W.groovy"
  echo "Windows build: tests and smoke test passed under Wine"
else
  echo "skipped: Windows build and tests (need x86_64-w64-mingw32-gcc, the $win target and Wine)"
fi

if [ -n "${JAVA_HOME:-}" ] || command -v java > /dev/null; then
  # Unit and TestKit tests, on the wrapper's Gradle and the oldest supported one. Needs JDK 17+.
  (cd gradle-plugin && ./gradlew --no-daemon check)
else
  echo "skipped: Gradle plugin tests (need a JDK 17 or later, on PATH or in JAVA_HOME)"
fi

if command -v npm > /dev/null && node -e 'process.exit(parseInt(process.versions.node) < 22 ? 1 : 0)'; then
  # The VS Code extension: lint, unit tests and the .vsix. ci.yml also runs its integration
  # tests in VS Code, trusted and in Restricted Mode, which download VS Code:
  # GROOVYFMT_BIN=$PWD/target/release/groovyfmt npm run test:integration
  # in editors/vscode (add VSCODE_TEST_ARGS=--ozone-platform=headless without a display).
  (cd editors/vscode && npm ci --no-audit --no-fund && npm run lint && npm run test:unit \
    && npm run package -- --out "$tmp/groovyfmt.vsix")
else
  echo "skipped: VS Code extension (needs Node.js 22 or later and npm)"
fi

# The JetBrains plugin builds with JDK 21. The Gradle plugin's oldest-Gradle tests can't run
# on 21, so a JDK 21 can be named apart from JAVA_HOME with JDK21_HOME.
java_major=$("${JAVA_HOME:+$JAVA_HOME/bin/}java" -XshowSettings:properties -version 2>&1 | sed -n 's/^ *java.specification.version = //p' || true)
if [ -n "${JDK21_HOME:-}" ] || [ "${java_major:-0}" -ge 21 ] 2> /dev/null; then
  # Unit tests, and integration tests in IntelliJ IDEA against this build (trusted,
  # untrusted, LSP4IJ's user-defined server). The first run downloads IntelliJ IDEA
  # Community (about 1 GB). ci.yml also runs the Plugin Verifier.
  (cd editors/jetbrains && GROOVYFMT_BIN="$PWD/../../$bin" ./gradlew --no-daemon --no-scan \
    ${JDK21_HOME:+"-Porg.gradle.java.installations.paths=$JDK21_HOME"} test)
else
  echo "skipped: JetBrains plugin tests (need a JDK 21 or later, in JDK21_HOME, JAVA_HOME or on PATH)"
fi

# Formatting a real corpus must stay idempotent; --stats exits 1 if any file is not.
if [ $# -gt 0 ]; then "$bin" --stats "$1" | tail -n 1; fi
echo "ci-local: ok"
