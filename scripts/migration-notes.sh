#!/usr/bin/env bash
# Write the migration part of a release's notes.
#
# Usage:
#
#     scripts/migration-notes.sh v0.1.5 notes.md
#
# Reads the notes in docs/migration/unreleased/ as the tag's tree holds them,
# which are the breaking changes that release carries. When there are any, the
# output file gets a heading naming the release they migrate from, a link to the
# migration guide the docs site renders from those notes, and the first line of
# each note as a list. When there are none, the output file is empty.
#
# The release it migrates from is the highest plain vX.Y.Z tag below this one,
# so the clone needs its tags.
#
# `.github/workflows/release.yml` passes the file to the release as its body,
# and GitHub puts the generated list of pull requests after it.

set -euo pipefail

if [ "$#" -ne 2 ] || [ -z "$1" ] || [ -z "$2" ]; then
  echo "usage: scripts/migration-notes.sh <tag> <out-file>" >&2
  exit 2
fi
tag="$1"
out="$2"
unreleased="docs/migration/unreleased"
guide="https://docs.lumenfx.dev/candela/migration/${tag}/"

if ! git rev-parse --quiet --verify "refs/tags/${tag}^{commit}" > /dev/null; then
  echo "migration-notes.sh: no tag ${tag} in this clone; fetch it first" >&2
  exit 1
fi

headings=()
while IFS= read -r -d '' entry; do
  kind="${entry#* }"
  kind="${kind%% *}"
  path="${entry#*$'\t'}"
  if [ "$kind" != "blob" ] || [ "${path##*/}" = ".gitkeep" ]; then
    continue
  fi
  first="$(git show "${tag}:${path}" | head -n 1)"
  if [ "${first#\# }" = "$first" ]; then
    # The tag cannot be changed any more, so a missing heading costs the list
    # its wording rather than stopping the release.
    echo "::warning::${path} does not start with a '# ' heading; listing it by its file name" >&2
    name="${path##*/}"
    headings+=("${name%.md}")
  else
    headings+=("${first#\# }")
  fi
done < <(git ls-tree -z "$tag" -- "${unreleased}/")

if [ "${#headings[@]}" -eq 0 ]; then
  : > "$out"
  exit 0
fi

previous="$(
  git tag --list 'v*' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | { cat; printf '%s\n' "$tag"; } \
    | sort -uV \
    | grep -B 1 -Fx "$tag" \
    | grep -vFx "$tag" \
    || true
)"

{
  if [ -n "$previous" ]; then
    printf '## Migrating from %s\n\n' "$previous"
  else
    printf '## Migrating to %s\n\n' "$tag"
  fi
  printf 'This release changes things you may have to act on. The [migration guide](%s) says what to do for each:\n\n' "$guide"
  for heading in "${headings[@]}"; do
    printf -- '- %s\n' "$heading"
  done
} > "$out"
