#!/usr/bin/env bash
# File the migration notes a release shipped under that release.
#
# Usage:
#
#     scripts/release-fragments.sh v0.1.5
#
# A pull request that breaks something adds a note to docs/migration/unreleased/.
# The tree a tag points at therefore holds, in that directory, the notes for the
# release the tag cuts. This moves exactly those notes, the ones the tag's tree
# lists, into docs/migration/<tag>/ in the working tree, with `git mv`, so the
# move is staged and the caller commits it. A note merged after the tag is not
# in the tag's tree, so it stays where it is and goes out with the next release.
#
# It moves only what is still in unreleased/, so running it again after the
# move landed does nothing. The tag has to be a ref this clone has.
#
# `.github/workflows/release.yml` runs this on main after a release publishes,
# in the same commit as the version bump.

set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "usage: scripts/release-fragments.sh <tag>" >&2
  exit 2
fi
tag="$1"
# A prerelease is never the release the docs are built from, so its notes stay
# unreleased and go out with the plain vX.Y.Z that follows it.
if ! printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "release-fragments.sh: ${tag} is not a plain vX.Y.Z tag; its notes stay in docs/migration/unreleased/" >&2
  exit 1
fi
unreleased="docs/migration/unreleased"
released="docs/migration/${tag}"

cd "$(git rev-parse --show-toplevel)"

if ! git rev-parse --quiet --verify "refs/tags/${tag}^{commit}" > /dev/null; then
  echo "release-fragments.sh: no tag ${tag} in this clone; fetch it first" >&2
  exit 1
fi

moved=0
while IFS= read -r -d '' entry; do
  # Each entry is "<mode> <type> <object>\t<path>".
  kind="${entry#* }"
  kind="${kind%% *}"
  path="${entry#*$'\t'}"
  name="${path##*/}"
  if [ "$kind" != "blob" ] || [ "$name" = ".gitkeep" ]; then
    continue
  fi
  if [ ! -e "$path" ]; then
    # Filed by an earlier run, or taken out of main since the tag.
    continue
  fi
  if [ -e "${released}/${name}" ]; then
    echo "release-fragments.sh: ${released}/${name} already exists, so ${path} has nowhere to go" >&2
    exit 1
  fi
  mkdir -p "$released"
  git mv "$path" "${released}/${name}"
  echo "moved ${path} to ${released}/${name}"
  moved=$(( moved + 1 ))
done < <(git ls-tree -z "$tag" -- "${unreleased}/")

echo "${moved} migration note(s) filed under ${tag}"
