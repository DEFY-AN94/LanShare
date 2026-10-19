#!/usr/bin/env bash
# Checks that a release tag matches the code, before the release workflow builds anything.
#
#   bash scripts/release_check.sh v0.2.0 [commit]
#
# The commit defaults to HEAD (in the release workflow, the tagged commit). Before tagging,
# run it against the commit you are about to tag: `git fetch && bash scripts/release_check.sh
# v0.2.0 origin/main`. In GitHub Actions it also writes its results to $GITHUB_OUTPUT.
#
# Rules (docs/releasing.md):
# - The tag is v<version in src-tauri/Cargo.toml>, optionally followed by a suffix such as
#   -rc.1. Cargo.toml is the only place that holds the version: Tauri falls back to it.
# - A tag without a suffix must be on main and needs docs/releases/v<version>.md.
#   A suffixed tag (a release candidate, or a test of the workflow) may be on any branch.
# - A suffixed tag, or any version below 1.0.0, is published as a pre-release.
set -euo pipefail

tag="${1:?usage: bash scripts/release_check.sh vX.Y.Z[-suffix] [commit]}"
commit=$(git rev-parse --verify "${2:-HEAD}^{commit}")

fail() {
  echo "::error::$*" >&2
  exit 1
}

[[ "$tag" =~ ^v([0-9]+)\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || fail "tag '$tag' is not vX.Y.Z or vX.Y.Z-suffix"
major="${BASH_REMATCH[1]}"
tag_version="${tag#v}"

version=$(git show "$commit:src-tauri/Cargo.toml" | sed -n 's/^version = "\(.*\)"$/\1/p' | head -1 || true)
[[ -n "$version" ]] || fail "no version in src-tauri/Cargo.toml"
if [[ "$tag_version" == "$version" ]]; then
  suffixed=false
elif [[ "$tag_version" == "$version"-* ]]; then
  suffixed=true
else
  fail "tag $tag does not match the version in src-tauri/Cargo.toml ($version)"
fi

lock_version=$(git show "$commit:src-tauri/Cargo.lock" | grep -A1 '^name = "lanshare"$' | sed -n 's/^version = "\(.*\)"$/\1/p' || true)
[[ "$lock_version" == "$version" ]] || fail "src-tauri/Cargo.lock still has lanshare $lock_version: run cargo check and commit Cargo.lock"

notes="docs/releases/v$version.md"
if ! git cat-file -e "$commit:$notes" 2>/dev/null; then
  $suffixed || fail "release notes $notes are missing"
  notes=""
fi

if ! $suffixed; then
  git fetch --quiet origin main
  git merge-base --is-ancestor "$commit" origin/main || fail "commit $commit is not on main"
fi

prerelease=false
if $suffixed || [[ "$major" == 0 ]] || [[ "$version" == *-* ]]; then
  prerelease=true
fi

echo "tag $tag: LanShare $version from commit $commit"
echo "pre-release: $prerelease; notes: ${notes:-none}"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "version=$version"
    echo "tag_version=$tag_version"
    echo "suffixed=$suffixed"
    echo "prerelease=$prerelease"
    echo "notes=$notes"
  } >> "$GITHUB_OUTPUT"
fi
