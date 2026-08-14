#!/bin/sh
# Candela toolchain installer.
#
#   curl -fsSL https://candela.lumenfx.dev/install.sh | sh
#
# Resolves a release of lumen-fx/candela (latest by default, or the tag given
# by --version), downloads the archive asset matching this platform, verifies
# it against the checksums published with the release, and unpacks it under
# ~/.candela. Nothing is written outside the prefix except an optional PATH
# line in a shell rc file, which is only added with consent. Nothing here
# needs root, and nothing here runs sudo.
#
# There is no separate manifest, no separate download host, and no API call:
# the release itself, at https://github.com/lumen-fx/candela/releases, is the
# source of both the archives and their checksums, and every request this
# script makes is a plain file download from
#
#   https://github.com/lumen-fx/candela/releases/download/<tag>/<asset>
#
# The latest tag comes from the redirect that
# https://github.com/lumen-fx/candela/releases/latest sends: its final URL ends
# in the tag, which is the same resolution candela's own update check uses
# (src/update.rs). A pinned --version needs no lookup at all; it becomes a tag
# directly, tried as given and then with a "v" prefix.
#
# Checksums live in one asset per release, sha256sums.txt, in sha256sum's own
# format: one "<hex>  <filename>" line per published asset. This script
# downloads it first, reads the line for the asset it wants, and refuses to
# install anything whose download does not match. A release that has no
# sha256sums.txt cannot be installed by this script.
#
# The asset naming below is the contract between .github/workflows/release.yml
# and this script:
#
#   candela-<os>-<arch>.tar.gz    os in {linux, macos}, arch in
#                                 {x86_64, aarch64}
#   candela-linux-x86_64-v3.tar.gz  the same build for CPUs with AVX2. The
#                                 unsuffixed asset is the baseline every
#                                 x86-64 machine can run; this one is taken
#                                 only when the CPU reports avx2 and the
#                                 release publishes it.
#   candela-windows-x86_64.msi    the Windows installer. This script never
#                                 fetches or runs it; the windows branch below
#                                 prints its URL and stops.
#   sha256sums.txt                checksums covering every asset above.
#
# Releases published before those names existed carry arch-first ones
# (candela-x86_64-linux-v1.tar.gz and so on) and are not renamed, so pinning
# to one fails with the names that release does publish.
#
# The archive holds the tree to install, flat: the candela compiler, the
# candela-vm runtime, and the standard library in libs/ beside them. Both
# binaries resolve `import "std/..."` relative to their own location, so the
# whole tree lands in one directory and that directory is what goes on PATH.
# The same layout is what the Windows package installs. Every installed path
# is recorded in a receipt at <prefix>/receipt, so a later run can replace an
# old version exactly and --uninstall can undo it.
#
# The receipt also records whether the install was pinned. With --version the
# receipt gets a "pinned <version>" line, and candela reads that line to stay
# quiet about newer releases: a pinned install is a deliberate choice, not
# something to nag about. Installing without --version rewrites the receipt
# without the line, which is how a pin is lifted.

set -eu

GH_REPO="${CANDELA_GH_REPO:-lumen-fx/candela}"
GH_URL="https://github.com/$GH_REPO"
PREFIX="${CANDELA_PREFIX:-$HOME/.candela}"

PIN_VERSION=""
NO_CONFIRM=0
MODIFY_PATH=1
FORCE=0
UNINSTALL=0

say() { printf '%s\n' "$*"; }
fail() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Candela toolchain installer.

Usage:
  install.sh [options]

Installs the candela compiler, the candela-vm runtime, and the standard
library, under a per-user prefix. Never uses sudo.

Options:
  --prefix DIR         Install root. Default: ~/.candela
  --version VERSION    Install a pinned release instead of the current one.
                       candela never offers to update a pinned install; run
                       the installer again without --version to lift the pin.
  --no-confirm         Run without prompting; still writes a PATH line to a
                       shell rc file unless --no-modify-path is also given.
  --no-modify-path     Never write a PATH line to a shell rc file.
  --force              Reinstall even if already at the target version.
  --uninstall          Remove every file this installer put under the prefix.
  -h, --help           Show this help.

Environment:
  CANDELA_GH_REPO  GitHub repo to install from, as owner/name.
                   Default: lumen-fx/candela
  CANDELA_PREFIX   Same as --prefix.
EOF
}

# --- arguments ---------------------------------------------------------------

while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix)
      [ "$#" -ge 2 ] || fail "--prefix needs a directory"
      PREFIX="$2"
      shift 2
      ;;
    --prefix=*) PREFIX="${1#--prefix=}"; shift ;;
    --version)
      [ "$#" -ge 2 ] || fail "--version needs a version"
      PIN_VERSION="$2"
      shift 2
      ;;
    --version=*) PIN_VERSION="${1#--version=}"; shift ;;
    --no-confirm) NO_CONFIRM=1; shift ;;
    --no-modify-path) MODIFY_PATH=0; shift ;;
    --force) FORCE=1; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown option: $1 (try --help)" ;;
  esac
done

[ -n "$PREFIX" ] || fail "--prefix needs a directory"

case "$PREFIX" in
  /*) ;;
  ~*) PREFIX="$HOME${PREFIX#\~}" ;;
  *) PREFIX="$PWD/$PREFIX" ;;
esac

# The binaries sit at the top of the prefix, next to the libs/ tree they read,
# so the prefix itself is the directory that goes on PATH.
BIN_DIR="$PREFIX"
RECEIPT="$PREFIX/receipt"

# --- tools -------------------------------------------------------------------

if command -v curl >/dev/null 2>&1; then
  DOWNLOADER=curl
elif command -v wget >/dev/null 2>&1; then
  DOWNLOADER=wget
else
  DOWNLOADER=none
fi

if command -v sha256sum >/dev/null 2>&1; then
  HASHER=sha256sum
elif command -v shasum >/dev/null 2>&1; then
  HASHER=shasum
else
  HASHER=none
fi

fetch_quiet() {
  # fetch_quiet URL DEST
  case "$DOWNLOADER" in
    curl) curl -fsSL -o "$2" "$1" ;;
    wget) wget -q -O "$2" "$1" ;;
    *) fail "need curl or wget" ;;
  esac
}

fetch_shown() {
  # fetch_shown URL DEST
  case "$DOWNLOADER" in
    curl) curl -fSL --progress-bar -o "$2" "$1" ;;
    wget) wget -O "$2" "$1" ;;
    *) fail "need curl or wget" ;;
  esac
}

sha256_of() {
  case "$HASHER" in
    sha256sum) sha256sum "$1" | cut -d' ' -f1 ;;
    shasum) shasum -a 256 "$1" | cut -d' ' -f1 ;;
    *) fail "need sha256sum or shasum to verify downloads" ;;
  esac
}

final_url() {
  # final_url URL -> the URL a GET of URL ends at, after redirects.
  case "$DOWNLOADER" in
    curl) curl -fsSL -o /dev/null -w '%{url_effective}' "$1" ;;
    wget)
      # --spider makes it a HEAD; --server-response writes every response
      # header to stderr, so the last Location is the end of the chain.
      wget --server-response --spider "$1" 2>&1 |
        awk 'tolower($1) == "location:" { print $2 }' |
        tail -n 1
      ;;
    *) fail "need curl or wget" ;;
  esac
}

# --- prompts -----------------------------------------------------------------

# Reads from the terminal, not stdin: with `curl ... | sh` stdin is the script
# itself. Without a terminal the answer is no, and --no-confirm is the way
# through.
ask() {
  if [ "$NO_CONFIRM" -eq 1 ]; then
    return 0
  fi
  # In a subshell: with no controlling terminal, opening /dev/tty is a fatal
  # redirection error in some shells, and the subshell contains it.
  if ! ( : >/dev/tty ) 2>/dev/null; then
    say "No terminal to ask on. Re-run with --no-confirm to accept the defaults."
    return 1
  fi
  printf '%s [Y/n] ' "$1" >/dev/tty
  ask_reply=""
  read -r ask_reply </dev/tty 2>/dev/null || ask_reply=n
  case "$ask_reply" in
    ''|y|Y|yes|Yes|YES) return 0 ;;
    *) return 1 ;;
  esac
}

# --- release data --------------------------------------------------------------
#
# Everything the script needs about a release comes out of its sha256sums.txt:
# which assets exist, and what each one hashes to. The file is sha256sum's own
# output, so a line is "<hex>  <filename>", and a filename may carry a leading
# "*" from binary mode.

asset_url() {
  # asset_url NAME -> the download URL for NAME in the resolved release
  printf '%s/releases/download/%s/%s\n' "$GH_URL" "$TAG" "$1"
}

asset_sha() {
  # asset_sha NAME -> the sha256 recorded for NAME, empty if it has no line
  awk -v want="$1" '
    NF >= 2 {
      name = $2
      sub(/^\*/, "", name)
      if (name == want) { print $1; exit }
    }' "$SUMS"
}

published_archives() {
  # published_archives -> one name per line for every candela-*.tar.gz the
  # release publishes, read off the checksum lines rather than a separate list.
  awk '
    NF >= 2 {
      name = $2
      sub(/^\*/, "", name)
      if (index(name, "candela-") != 1) { next }
      if (name !~ /\.tar\.gz$/) { next }
      print name
    }' "$SUMS"
}

# --- receipt -------------------------------------------------------------------
#
#   version 0.1.0
#   target linux-x86_64
#   pinned 0.1.0
#   file candela
#   file candela-vm
#   file libs/std/list.cdl
#
# The "pinned" line is present only for a --version install, and carries the
# resolved release, so it always agrees with the "version" line above it.
# candela's update check (src/update.rs) treats its presence as "leave this
# install alone".

receipt_version() {
  [ -f "$RECEIPT" ] || return 0
  awk '$1 == "version" { print $2; exit }' "$RECEIPT"
}

receipt_files() {
  [ -f "$RECEIPT" ] || return 0
  awk '$1 == "file" { print substr($0, 6) }' "$RECEIPT"
}

set_receipt_pin() {
  # set_receipt_pin VERSION|"" -> rewrite an existing receipt with, or
  # without, its "pinned" line and leave every other line alone. Used on the
  # already-up-to-date path, where nothing else is rewritten but the pin still
  # has to follow the flags this run was given.
  [ -f "$RECEIPT" ] || return 0
  srp_tmp="$RECEIPT.tmp.$$"
  {
    awk '$1 != "pinned" && $1 != "file"' "$RECEIPT"
    if [ -n "$1" ]; then
      printf 'pinned %s\n' "$1"
    fi
    awk '$1 == "file"' "$RECEIPT"
  } > "$srp_tmp"
  mv "$srp_tmp" "$RECEIPT"
}

prune_dirs() {
  # Removes directories left empty by a removal. rmdir refuses non-empty ones.
  [ -d "$PREFIX" ] || return 0
  find "$PREFIX" -depth -type d -exec rmdir {} + 2>/dev/null || true
}

# --- uninstall ---------------------------------------------------------------

do_uninstall() {
  if [ ! -f "$RECEIPT" ]; then
    say "Nothing to uninstall: no Candela install found at $PREFIX"
    exit 0
  fi

  say "Removing from $PREFIX:"
  say "  candela $(receipt_version)"
  if ! ask "Remove these?"; then
    say "Cancelled."
    exit 1
  fi

  receipt_files | while IFS= read -r rel; do
    [ -n "$rel" ] || continue
    rm -f "$PREFIX/$rel"
  done
  rm -f "$RECEIPT"
  prune_dirs
  say "Removed. If a PATH line for $BIN_DIR is still in a shell rc file, delete it by hand."
  exit 0
}

if [ "$UNINSTALL" -eq 1 ]; then
  do_uninstall
fi

# --- platform ----------------------------------------------------------------

UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"

case "$UNAME_S" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) OS=windows ;;
  *) fail "unsupported operating system: $UNAME_S. Candela ships for Linux and macOS." ;;
esac

case "$UNAME_M" in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) fail "unsupported architecture: $UNAME_M. Candela ships for x86_64 and aarch64." ;;
esac

TARGET="$OS-$ARCH"

[ "$DOWNLOADER" != none ] || fail "need curl or wget"
[ "$HASHER" != none ] || fail "need sha256sum or shasum to verify downloads"

# --- resolve the release ------------------------------------------------------

TMP="$(mktemp -d "${TMPDIR:-/tmp}/candela-install.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

SUMS="$TMP/sha256sums.txt"

sums_missing() {
  fail "release $1 of $GH_REPO has no sha256sums.txt. Either that release does not exist, or it predates checksum publishing and this installer cannot verify it. See $GH_URL/releases"
}

# A pinned version is tried as given, then with a "v" prefix, since this
# project tags releases vX.Y.Z but --version is documented as taking the bare
# number. The checksum file is the probe: a tag with no sha256sums.txt behind
# it is a tag this script cannot install from, whatever the reason.
if [ -n "$PIN_VERSION" ]; then
  TAG="$PIN_VERSION"
  # The first attempt is a guess at which of the two tag shapes this is, so
  # its failure is not news: keep the downloader quiet about it and let the
  # retry, or the message below, do the talking.
  if ! fetch_quiet "$(asset_url sha256sums.txt)" "$SUMS" 2>/dev/null; then
    case "$PIN_VERSION" in
      v*) sums_missing "$PIN_VERSION" ;;
      *)
        TAG="v$PIN_VERSION"
        fetch_quiet "$(asset_url sha256sums.txt)" "$SUMS" ||
          sums_missing "$PIN_VERSION (tried tags $PIN_VERSION and $TAG)"
        ;;
    esac
  fi
else
  # /releases/latest redirects to /releases/tag/<tag>, so the last path
  # segment of the final URL is the tag. With no releases at all the redirect
  # lands on the release index instead, which is what the guard below catches.
  LATEST_URL="$(final_url "$GH_URL/releases/latest" || true)"
  TAG="${LATEST_URL##*/}"
  case "$TAG" in
    ''|latest|releases)
      fail "could not resolve the latest release of $GH_REPO. Either it has no releases yet, or the request did not get through. See $GH_URL/releases"
      ;;
  esac
  fetch_quiet "$(asset_url sha256sums.txt)" "$SUMS" || sums_missing "$TAG"
fi

[ -s "$SUMS" ] || sums_missing "$TAG"
RELEASE="${TAG#v}"

if [ "$OS" = windows ]; then
  say "This installer covers Linux and macOS."
  if [ -n "$(asset_sha "candela-windows-$ARCH.msi")" ]; then
    say "For Windows, download and run the installer:"
    say "  $(asset_url "candela-windows-$ARCH.msi")"
  else
    say "A Windows installer is not published for $ARCH yet. See $GH_URL/releases"
  fi
  exit 1
fi

# --- resolve the asset ---------------------------------------------------------

# The baseline asset is the one every machine of this architecture can run.
# On x86-64 Linux a CPU with AVX2 takes the -v3 build instead, when the release
# publishes one.
ASSET_NAME="candela-$TARGET.tar.gz"
if [ "$TARGET" = linux-x86_64 ] && grep -q avx2 /proc/cpuinfo 2>/dev/null; then
  if [ -n "$(asset_sha "candela-$TARGET-v3.tar.gz")" ]; then
    ASSET_NAME="candela-$TARGET-v3.tar.gz"
  fi
fi

ASSET_SHA="$(asset_sha "$ASSET_NAME")"
if [ -z "$ASSET_SHA" ]; then
  fail "release $TAG of $GH_REPO publishes no $ASSET_NAME. It publishes: $(published_archives | tr '\n' ' ' | sed 's/ *$//'). Releases made before the asset names changed keep their old names and cannot be installed by this script. See $GH_URL/releases"
fi

INSTALLED="$(receipt_version)"
if [ "$FORCE" -eq 0 ] && [ "$INSTALLED" = "$RELEASE" ]; then
  # Nothing to copy, but the pin still follows this run's flags: --version on
  # the version already installed pins it, and a plain re-run lifts a pin.
  if [ -n "$PIN_VERSION" ]; then
    set_receipt_pin "$RELEASE"
  else
    set_receipt_pin ""
  fi
  say ""
  say "Candela toolchain installer"
  say ""
  say "  release   $RELEASE"
  say "  target    $TARGET"
  say "  prefix    $PREFIX"
  say ""
  say "Already up to date: candela $INSTALLED"
  if [ -n "$PIN_VERSION" ]; then
    say "Pinned to $RELEASE. candela will not offer newer releases."
  fi
  say ""
  say "Use --force to reinstall."
  exit 0
fi

say ""
say "Candela toolchain installer"
say ""
say "  release   $RELEASE"
say "  target    $TARGET"
say "  prefix    $PREFIX"
say ""
if [ -n "$INSTALLED" ]; then
  say "  candela $INSTALLED -> $RELEASE"
else
  say "  candela $RELEASE"
fi
say "    the candela compiler, candela-vm, and the standard library"
say ""

if ! ask "Install?"; then
  say "Cancelled. Nothing was written."
  exit 1
fi

# --- download and verify -----------------------------------------------------

ASSET_URL="$(asset_url "$ASSET_NAME")"

say "Downloading candela"
mkdir -p "$TMP/dl"
if ! fetch_shown "$ASSET_URL" "$TMP/dl/candela.tar.gz"; then
  fail "download failed: $ASSET_URL"
fi

got="$(sha256_of "$TMP/dl/candela.tar.gz")"
if [ "$got" != "$ASSET_SHA" ]; then
  fail "checksum mismatch for candela
  expected $ASSET_SHA
  got      $got
Nothing was installed. The download was corrupted, or the asset at $ASSET_URL does not match the checksum published with release $TAG."
fi

# --- unpack and install ------------------------------------------------------

root="$TMP/x"
mkdir -p "$root"
tar -xzf "$TMP/dl/candela.tar.gz" -C "$root" || fail "could not unpack the candela archive"

# Tolerate one wrapping directory inside the archive.
if [ ! -f "$root/candela" ]; then
  inner=""
  inner_count=0
  for candidate in "$root"/*; do
    [ -e "$candidate" ] || continue
    inner_count=$((inner_count + 1))
    inner="$candidate"
  done
  if [ "$inner_count" -eq 1 ] && [ -f "$inner/candela" ]; then
    root="$inner"
  fi
fi
[ -f "$root/candela" ] || fail "the candela archive has no candela binary in it"
[ -f "$root/candela-vm" ] || fail "the candela archive has no candela-vm binary in it"
# `import "std/..."` resolves relative to the binary, so the library has to
# travel with it.
[ -d "$root/libs/std" ] || fail "the candela archive has no libs/std directory"

( cd "$root" && find . \( -type f -o -type l \) -print ) | sed 's|^\./||' | sort > "$TMP/files"
[ -s "$TMP/files" ] || fail "the candela archive is empty"

say "Installing candela $RELEASE"
while IFS= read -r rel; do
  dest="$PREFIX/$rel"
  mkdir -p "$(dirname "$dest")"
  rm -f "$dest"
  cp -p "$root/$rel" "$dest"
done < "$TMP/files"

# Files the previous version installed and this one does not.
receipt_files | sort > "$TMP/old" || true
if [ -s "$TMP/old" ]; then
  comm -23 "$TMP/old" "$TMP/files" | while IFS= read -r stale; do
    [ -n "$stale" ] || continue
    rm -f "$PREFIX/$stale"
  done
fi

{
  printf 'version %s\n' "$RELEASE"
  printf 'target %s\n' "$TARGET"
  if [ -n "$PIN_VERSION" ]; then
    printf 'pinned %s\n' "$RELEASE"
  fi
  sed 's/^/file /' "$TMP/files"
} > "$RECEIPT"

chmod 755 "$PREFIX/candela" "$PREFIX/candela-vm"

prune_dirs

# --- PATH --------------------------------------------------------------------

# The rc line keeps $PATH unexpanded on purpose: it is written to the file
# verbatim and expanded by the shell that reads it.
# shellcheck disable=SC2016
path_line_for() {
  case "$1" in
    */fish) printf 'set -gx PATH "%s" $PATH\n' "$BIN_DIR" ;;
    *) printf 'export PATH="%s:$PATH"\n' "$BIN_DIR" ;;
  esac
}

rc_file_for() {
  case "$1" in
    */fish) printf '%s\n' "$HOME/.config/fish/config.fish" ;;
    */zsh) printf '%s\n' "$HOME/.zshrc" ;;
    */bash)
      if [ "$OS" = macos ] && [ -f "$HOME/.bash_profile" ]; then
        printf '%s\n' "$HOME/.bash_profile"
      else
        printf '%s\n' "$HOME/.bashrc"
      fi
      ;;
    *) printf '%s\n' "$HOME/.profile" ;;
  esac
}

on_path=0
case ":$PATH:" in
  *":$BIN_DIR:"*) on_path=1 ;;
esac

say ""
if [ "$on_path" -eq 0 ]; then
  RC="$(rc_file_for "${SHELL:-/bin/sh}")"
  LINE="$(path_line_for "${SHELL:-/bin/sh}")"
  already=0
  if [ -f "$RC" ] && grep -q -F "$BIN_DIR" "$RC" 2>/dev/null; then
    already=1
  fi
  if [ "$already" -eq 1 ]; then
    say "$BIN_DIR is already in $RC. Open a new shell to pick it up."
  elif [ "$MODIFY_PATH" -eq 0 ]; then
    say "Add $BIN_DIR to your PATH:"
    say "  $LINE"
  elif ask "Add $BIN_DIR to your PATH in $RC?"; then
    mkdir -p "$(dirname "$RC")"
    {
      printf '\n# added by the Candela installer\n'
      printf '%s\n' "$LINE"
    } >> "$RC"
    say "Added to $RC. Open a new shell, or run:"
    say "  $LINE"
  else
    say "Left your shell configuration alone. To use Candela, add:"
    say "  $LINE"
  fi
fi

say ""
say "Installed under $PREFIX:"
say "  candela $(receipt_version)"
say "  candela-vm, and the standard library in libs/"
if [ -n "$PIN_VERSION" ]; then
  say ""
  say "Pinned to $RELEASE. candela will not offer newer releases; re-run this"
  say "installer without --version to lift the pin."
fi
say ""
say "Get started:"
say "  candela"
