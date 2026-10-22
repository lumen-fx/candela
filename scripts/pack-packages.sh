#!/usr/bin/env bash
# Pack the first-party packages under packages/ for one target.
#
# Usage:
#
#     scripts/pack-packages.sh <target> <out-dir> [<library-dir>]
#
# For a desktop target (linux-x86_64, linux-aarch64, macos-x86_64,
# macos-aarch64, windows-x86_64) this packs every package with a native half,
# taking the library cargo built for it from <library-dir> and shipping it as
# dist/<name>.<ext>, the unsuffixed path the package's `dylib` block names. For
# `any` it packs the packages with no native half, and needs no library.
#
# Each archive is <out-dir>/<name>-<target>.tar.gz, with the package root at
# the root of the archive: candela.toml, src/, dist/ when there is a native
# half, and web/ when the manifest names a browser half there. That is the
# shape build-package.yml packs a package in, so the registry client unpacks
# these the way it unpacks any other.

set -euo pipefail

target="${1:?usage: pack-packages.sh <target> <out-dir> [<library-dir>]}"
out="${2:?usage: pack-packages.sh <target> <out-dir> [<library-dir>]}"
libs="${3:-}"
root="$(cd "$(dirname "$0")/.." && pwd)"

case "$target" in
  linux-*) prefix=lib ext=so ;;
  macos-*) prefix=lib ext=dylib ;;
  windows-*) prefix="" ext=dll ;;
  any) prefix="" ext="" ;;
  *)
    echo "pack-packages.sh: unknown target $target" >&2
    exit 1
    ;;
esac
if [ "$target" != any ] && [ -z "$libs" ]; then
  echo "pack-packages.sh: $target needs the directory cargo built the libraries in" >&2
  exit 1
fi

mkdir -p "$out"
out="$(cd "$out" && pwd)"
packed=0
for package in "$root"/packages/*/; do
  package="${package%/}"
  manifest="$package/candela.toml"
  [ -f "$manifest" ] || continue
  name="$(basename "$package")"
  native=false
  [ -d "$package/native" ] && native=true
  if [ "$target" = any ] && $native; then continue; fi
  if [ "$target" != any ] && ! $native; then continue; fi

  stage="$(mktemp -d)"
  cp "$manifest" "$stage/"
  cp -R "$package/src" "$stage/"
  members=(candela.toml src)
  if [ -d "$package/web" ]; then
    cp -R "$package/web" "$stage/"
    members+=(web)
  fi
  if $native; then
    library="$libs/${prefix}candela_${name}.${ext}"
    if [ ! -f "$library" ]; then
      echo "pack-packages.sh: no $library for $name on $target" >&2
      exit 1
    fi
    mkdir -p "$stage/dist"
    cp "$library" "$stage/dist/$name.$ext"
    members+=(dist)
  fi
  archive="$out/$name-$target.tar.gz"
  tar -czf "$archive" -C "$stage" "${members[@]}"
  rm -rf "$stage"
  echo "$archive"
  tar -tzf "$archive" | sed 's/^/  /'
  packed=$((packed + 1))
done

if [ "$packed" -eq 0 ]; then
  echo "pack-packages.sh: nothing to pack for $target" >&2
  exit 1
fi
