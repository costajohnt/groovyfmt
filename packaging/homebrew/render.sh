#!/usr/bin/env bash
# Prints the Homebrew formula for a release:
#   packaging/homebrew/render.sh <version> <dir-with-sha256-files>
# The directory holds groovyfmt-v<version>-<target>.tar.gz.sha256 for each
# target (gh release download v<version> --pattern '*.sha256').
set -euo pipefail

version=${1:?usage: render.sh <version> <dir-with-sha256-files>}
dir=${2:?usage: render.sh <version> <dir-with-sha256-files>}
version=${version#v}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "render.sh: not a version: $version" >&2
  exit 2
fi

formula=$(<"$(dirname "$0")/groovyfmt.rb.tmpl")
formula=${formula//@VERSION@/$version}
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  sha=$(cut -d ' ' -f 1 "$dir/groovyfmt-v$version-$target.tar.gz.sha256")
  if ! [[ $sha =~ ^[0-9a-f]{64}$ ]]; then
    echo "render.sh: no sha256 in $dir/groovyfmt-v$version-$target.tar.gz.sha256" >&2
    exit 1
  fi
  formula=${formula//@SHA256_$target@/$sha}
done
printf '%s\n' "$formula"
