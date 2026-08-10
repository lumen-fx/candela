#!/bin/sh

# CANDELA INSTALLER
#
# Every request is a plain file download from the release itself, at
# https://github.com/lumen-fx/candela/releases. Each release publishes
# sha256sums.txt, one "<hex>  <filename>" line per asset. This script downloads
# that file first, reads the line for the archive it wants, and installs
# nothing whose download does not match.

set -e

usage() {
    printf "Usage: install.sh [--version TAG]\n"
    printf "  --version TAG   Install a specific release, e.g. 0.3.0 or v0.3.0.\n"
    printf "                  Without it, the latest release is installed.\n"
}

missing_tag() {
    printf "[ERROR] --version needs a release tag\n" >&2
    usage >&2
    exit 1
}

# The release tag to install. Empty means "whatever is latest right now".
PIN=""

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            shift
            [ $# -gt 0 ] || missing_tag
            PIN="$1"
            [ -n "$PIN" ] || missing_tag
            ;;
        --version=*)
            PIN="${1#--version=}"
            [ -n "$PIN" ] || missing_tag
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            printf "[ERROR] Unknown option: %s\n" "$1" >&2
            usage >&2
            exit 1
            ;;
    esac
    shift
done

# Supported OS's: "Darwin" on macOS, "Linux" on Linux
OS=$(uname -s)

case "$OS" in
    Darwin) INSTALL_DIR="/Library/Candela/" ;;
    Linux) INSTALL_DIR="/usr/local/lib/candela/" ;;
esac

if mkdir -p "$INSTALL_DIR" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir "$INSTALL_DIR"
else
    printf "[ERROR] Cannot write to %s and sudo is not available. Re-run as root or install sudo.\n" "$INSTALL_DIR"
fi

if command -v curl >/dev/null 2>&1; then
    DOWNLOADER=curl
elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER=wget
else
    printf "[ERROR] curl or wget is required\n" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    HASHER=sha256sum
elif command -v shasum >/dev/null 2>&1; then
    HASHER=shasum
else
    printf "[ERROR] sha256sum or shasum is required to verify the download\n" >&2
    exit 1
fi

# Write to a file rather than stdout: the archive is verified before it is
# unpacked, so it has to land somewhere first.
download() {
    # download URL DEST
    case "$DOWNLOADER" in
        # Fail on HTTP errors, show errors even when silent, follow redirects,
        # show a progress bar.
        curl) curl -fSL --progress-bar -o "$2" "$1" ;;
        wget) wget -O "$2" "$1" ;;
    esac
}

download_quiet() {
    # download_quiet URL DEST
    case "$DOWNLOADER" in
        curl) curl -fsSL -o "$2" "$1" ;;
        wget) wget -q -O "$2" "$1" ;;
    esac
}

sha256_of() {
    case "$HASHER" in
        sha256sum) sha256sum "$1" | cut -d' ' -f1 ;;
        shasum) shasum -a 256 "$1" | cut -d' ' -f1 ;;
    esac
}

# Supported archs: x86_64, arm64, aarch64
ARCH=$(uname -m)

case "$OS" in
    Darwin)
        case "$ARCH" in
            x86_64)  ARTIFACT="candela-x86_64-apple-darwin" ;;
            arm64)   ARTIFACT="candela-aarch64-apple-darwin" ;;
            *)       printf "[ERROR] Unsupported macOS architecture: %s\n" "$ARCH" >&2; exit 1 ;;
        esac
        ;;
    Linux)
        case "$ARCH" in
            x86_64)
                # Quietly check if AVX2 is supported by the current CPU on Linux
                if grep -q avx2 /proc/cpuinfo 2>/dev/null; then
                    ARTIFACT="candela-x86_64-linux-v3"
                else
                    # Fallback for older CPUs -> Candela will probabky be slower
                    ARTIFACT="candela-x86_64-linux-v1"
                fi
                ;;
            aarch64) ARTIFACT="candela-aarch64-linux" ;;
            *)       printf "[ERROR] Unsupported Linux architecture: %s\n" "$ARCH" >&2; exit 1 ;;
        esac
        ;;
    *)
        # Windows installs from its own package instead of this script.
        printf "[ERROR] Unsupported OS: %s. On Windows, run https://github.com/lumen-fx/candela/releases/latest/download/candela-x86_64-windows.msi\n" "$OS" >&2
        exit 1
        ;;
esac

printf "[Candela] Ground Control to Major Tom...\n"
printf "[Candela] Downloading %s for %s/%s\n" "$ARTIFACT" "$OS" "$ARCH"

TMP=$(mktemp -d)

# Clean up the temp directory once the script exits, for ANY reason
trap 'rm -rf "$TMP"' EXIT

RELEASES="https://github.com/lumen-fx/candela/releases"

SUMS="$TMP/sha256sums.txt"
ARCHIVE="$TMP/$ARTIFACT.tar.gz"
UNPACK="$TMP/unpack"

sums_missing() {
    printf "[ERROR] Release %s has no sha256sums.txt, so the download cannot be verified. Either that release does not exist, or it predates checksum publishing. See %s\n" "$1" "$RELEASES" >&2
    exit 1
}

PIN_VERSION="${PIN#v}"

# The checksum file settles which release this is: it names every asset the
# release publishes and what each one hashes to, and a tag without one is a tag
# this script cannot install from. Release tags carry a leading "v", so a bare
# 0.3.0 is tried as given and then with the prefix.
if [ -z "$PIN" ]; then
    LABEL="latest"
    BASE="$RELEASES/latest/download"
    download_quiet "$BASE/sha256sums.txt" "$SUMS" || sums_missing "$LABEL"
else
    LABEL="$PIN"
    BASE="$RELEASES/download/$PIN"
    if download_quiet "$BASE/sha256sums.txt" "$SUMS" 2>/dev/null; then
        :
    elif [ "$PIN" != "v$PIN_VERSION" ]; then
        BASE="$RELEASES/download/v$PIN_VERSION"
        download_quiet "$BASE/sha256sums.txt" "$SUMS" ||
            sums_missing "$PIN (tried tags $PIN and v$PIN_VERSION)"
    else
        sums_missing "$PIN"
    fi
fi

[ -s "$SUMS" ] || sums_missing "$LABEL"

# One "<hex>  <filename>" line per asset, where the name may carry a leading "*"
# from sha256sum's binary mode.
EXPECTED=$(awk -v want="$ARTIFACT.tar.gz" '
    NF >= 2 {
        name = $2
        sub(/^\*/, "", name)
        if (name == want) { print $1; exit }
    }' "$SUMS")

if [ -z "$EXPECTED" ]; then
    printf "[ERROR] No %s archive in that release. See %s\n" "$ARTIFACT" "$RELEASES" >&2
    exit 1
fi

if ! download "$BASE/$ARTIFACT.tar.gz" "$ARCHIVE"; then
    printf "[ERROR] Could not download %s. See %s\n" "$ARTIFACT" "$RELEASES" >&2
    exit 1
fi

GOT=$(sha256_of "$ARCHIVE")

if [ "$GOT" != "$EXPECTED" ]; then
    printf "[ERROR] Checksum mismatch for %s\n" "$ARTIFACT" >&2
    printf "        expected %s\n" "$EXPECTED" >&2
    printf "        got      %s\n" "$GOT" >&2
    printf "        Nothing was installed. The download was corrupted, or the archive does not match the checksum published with the release.\n" >&2
    exit 1
fi

printf "[Candela] Checksum verified\n"

mkdir -p "$UNPACK"

if ! tar -xzf "$ARCHIVE" -C "$UNPACK"; then
    printf "[ERROR] Could not unpack the %s archive. See %s\n" "$ARTIFACT" "$RELEASES" >&2
    exit 1
fi

if [ ! -f "$UNPACK/candela" ]; then
    # The github workflow packs the binary straight into an archive so something went very wrong here
    printf "[ERROR] Archive downloaded but binary not found inside. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

if [ ! -f "$UNPACK/candela-vm" ]; then
    # The release archive ships the VM-only runtime (candela-vm) alongside the
    # full compiler (candela) so a `.cdlb` can run without the compiler.
    printf "[ERROR] Archive downloaded but candela-vm not found inside. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

if [ ! -d "$UNPACK/libs/std" ]; then
    # The archive ships the standard library in libs/ next to the binary. `import
    # std::x` resolves relative to the installed binary, so this must be present.
    printf "[ERROR] Archive downloaded but the standard library (libs/std) is missing. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

# Copy the whole unpacked tree, so the binary AND the libs/ tree (holding the std
# library) land together in INSTALL_DIR. The binary resolves `import std::x`
# relative to its own location, so libs/ must sit beside it: INSTALL_DIR/candela
# and INSTALL_DIR/libs/std are the single source of truth for that lookup.
if cp -R "$UNPACK/." "$INSTALL_DIR" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo cp -R "$UNPACK/." "$INSTALL_DIR"
else
    printf "[ERROR] Cannot write to %s and sudo is not available. Re-run as root or install sudo.\n" "$INSTALL_DIR"
fi

if [ ! -d "$INSTALL_DIR/libs/std" ]; then
    printf "[ERROR] Standard library not installed at %s/libs/std. 'import std::x' will not resolve. Please re-run the installer.\n" "$INSTALL_DIR"
fi

if chmod 755 "$INSTALL_DIR/candela" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo chmod 755 "$INSTALL_DIR/candela"
fi

if chmod 755 "$INSTALL_DIR/candela-vm" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo chmod 755 "$INSTALL_DIR/candela-vm"
fi

if ln -sf "$INSTALL_DIR/candela" /usr/local/bin/candela 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo ln -sf "$INSTALL_DIR/candela" /usr/local/bin/candela
else
    printf "[ERROR] Cannot write to /usr/local/bin and sudo is not available. Re-run as root or install sudo.\n"
fi

if ln -sf "$INSTALL_DIR/candela-vm" /usr/local/bin/candela-vm 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo ln -sf "$INSTALL_DIR/candela-vm" /usr/local/bin/candela-vm
else
    printf "[ERROR] Cannot write to /usr/local/bin and sudo is not available. Re-run as root or install sudo.\n"
fi

VERSION=$("$INSTALL_DIR/candela" --version | cut -d' ' -f2)

# The receipt records what this installer put in place. `candela` reads it to
# decide whether to check for a newer release: no receipt means the binary was
# built from source and is left alone, and a `pinned` line means you chose this
# release and do not want to hear about newer ones. Installing without
# --version rewrites the receipt without that line, which lifts the pin.
printf "version %s\n" "$VERSION" > "$TMP/receipt"
if [ -n "$PIN" ]; then
    printf "pinned %s\n" "$PIN_VERSION" >> "$TMP/receipt"
fi

if cp "$TMP/receipt" "$INSTALL_DIR/receipt" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo cp "$TMP/receipt" "$INSTALL_DIR/receipt"
fi

printf "[Candela] Installed Candela %s in %s/candela\n" "$VERSION" "$INSTALL_DIR"
printf "[Candela] Installed candela-vm in %s/candela-vm\n" "$INSTALL_DIR"
printf "[Candela] Run 'candela' to get started.\n"
