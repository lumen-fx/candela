#!/usr/bin/env bash
# Lay out the `libs` directory a toolchain archive carries.
#
# Usage:
#
#     scripts/stage-std.sh <target> <out-dir> <library-dir>
#
# <target> is linux-*, macos-* or windows-*, which names the library suffix.
# <library-dir> is where cargo built the standard library's Rust crates for
# that target (target/<triple>/release).
#
# <out-dir>/libs gets libs/std whole, and one directory per native library
# under std_src/: the C libraries as the build left them in libs/std_src
# (sources and the library the build step compiled beside them), and each
# Rust cdylib crate's built library as std_src/<name>/<name>.<ext>, the path
# the std modules' `dylib` blocks name. The Rust crates' sources, and the
# crates that build no library (sys, testkit), stay out.

set -euo pipefail

target="${1:?usage: stage-std.sh <target> <out-dir> <library-dir>}"
out="${2:?usage: stage-std.sh <target> <out-dir> <library-dir>}"
built="${3:?usage: stage-std.sh <target> <out-dir> <library-dir>}"
root="$(cd "$(dirname "$0")/.." && pwd)"

case "$target" in
  linux-*) prefix=lib ext=so ;;
  macos-*) prefix=lib ext=dylib ;;
  windows-*) prefix="" ext=dll ;;
  *)
    echo "stage-std.sh: unknown target $target" >&2
    exit 1
    ;;
esac

libs="$out/libs"
mkdir -p "$libs/std_src"
cp -R "$root/libs/std" "$libs/"
for dir in "$root"/libs/std_src/*/; do
  dir="${dir%/}"
  name="$(basename "$dir")"
  manifest="$dir/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    cp -R "$dir" "$libs/std_src/"
    continue
  fi
  grep -q 'crate-type = \[.*"cdylib"' "$manifest" || continue
  library="$built/${prefix}candela_std_${name}.${ext}"
  if [ ! -f "$library" ]; then
    echo "stage-std.sh: no $library for $name on $target" >&2
    exit 1
  fi
  mkdir -p "$libs/std_src/$name"
  cp "$library" "$libs/std_src/$name/$name.$ext"
done
find "$libs/std_src" -type f -name "*.$ext" | sort
