#!/usr/bin/env bash
# Fails unless the pushed tag (GITHUB_REF_NAME) is v<Cargo.toml version>, the VS Code
# extension has the same version, the checked-out commit is on origin/main, and
# CHANGELOG.md has a heading for the version. Run from the repository root with full history fetched.
set -euo pipefail

tag=${GITHUB_REF_NAME:?GITHUB_REF_NAME is not set}
version=$(cargo metadata --no-deps --format-version 1 --locked |
  jq -r '.packages[] | select(.name == "groovyfmt") | .version')

if [ "$tag" != "v$version" ]; then
  echo "::error::Tag $tag does not match the Cargo.toml version $version (expected v$version)"
  exit 1
fi
vscode=$(jq -r .version editors/vscode/package.json)
if [ "$vscode" != "$version" ]; then
  echo "::error::editors/vscode/package.json has version $vscode, expected $version"
  exit 1
fi
if ! git merge-base --is-ancestor HEAD origin/main; then
  echo "::error::Tag $tag points at $(git rev-parse HEAD), which is not on origin/main"
  exit 1
fi
if ! grep -qE "^## ${version//./\\.}( |$)" CHANGELOG.md; then
  echo "::error::CHANGELOG.md has no '## $version' heading"
  exit 1
fi
echo "Tag $tag matches Cargo.toml $version, is on main, and has a CHANGELOG entry"
