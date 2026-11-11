#!/usr/bin/env bash
# Builds the Docker build context for the Dockerfile at the repository root:
#   packaging/docker/context.sh <archives-dir> <out-dir>
# <archives-dir> holds the two linux-musl release archives and their .sha256
# files; each is checked before it is unpacked to <out-dir>/<amd64|arm64>/.
set -euo pipefail

archives=${1:?usage: context.sh <archives-dir> <out-dir>}
out=${2:?usage: context.sh <archives-dir> <out-dir>}
mkdir -p "$out"
cp "$(dirname "$0")/../../LICENSE" "$out/"
for pair in amd64:x86_64-unknown-linux-musl arm64:aarch64-unknown-linux-musl; do
  arch=${pair%%:*}
  target=${pair#*:}
  matches=("$archives"/groovyfmt-v*-"$target".tar.gz)
  if [ "${#matches[@]}" -ne 1 ] || [ ! -f "${matches[0]}" ]; then
    echo "context.sh: want exactly one $target archive in $archives" >&2
    exit 1
  fi
  archive=${matches[0]}
  expected=$(cut -d ' ' -f 1 "$archive.sha256")
  actual=$(shasum -a 256 "$archive" | cut -d ' ' -f 1)
  if [ "$expected" != "$actual" ]; then
    echo "context.sh: checksum mismatch for $archive: expected $expected, got $actual" >&2
    exit 1
  fi
  mkdir -p "$out/$arch"
  tar -xzf "$archive" -C "$out/$arch" --strip-components=1
done
