#!/bin/sh

# CANDELA INSTALLER

set -e

# Supported OS's: "Darwin" on macOS, "Linux" on Linux
OS=$(uname -s)

case "$OS" in
    Darwin) INSTALL_DIR="/Library/Candela/" ;;
    Linux) INSTALL_DIR="/usr/local/lib/candela/" ;;
esac

if mkdir -p "$INSTALL_DIR" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir $INSTALL_DIR
else
    printf "[ERROR] Cannot write to $INSTALL_DIR and sudo is not available. Re-run as root or install sudo.\n"
fi

if command -v curl >/dev/null 2>&1; then
    # Fail silently on HTTP errors & show errors even when silent & follow redirects & show progress bar
    DOWNLOAD_CMD="curl -fSL --progress-bar"
elif command -v wget >/dev/null 2>&1; then
    # Write output to stdout & show progress bar
    DOWNLOAD_CMD="wget -O- --show-progress"
else
    printf "[ERROR] curl or wget is required\n"
fi

# Supported archs: x86_64, arm64, aarch64
ARCH=$(uname -m)

case "$OS" in
    Darwin)
        case "$ARCH" in
            x86_64)  ARTIFACT="candela-x86_64-apple-darwin" ;;
            arm64)   ARTIFACT="candela-aarch64-apple-darwin" ;;
            *)       printf "[ERROR] Unsupported macOS architecture: $ARCH\n" ;;
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
            *)       printf "[ERROR] Unsupported Linux architecture: $ARCH\n" ;;
        esac
        ;;
    *)
        # Windows will eventually be supported by an installer
        printf "[ERROR] Unsupported OS: $OS. On Windows, download the .zip from https://github.com/lumen-fx/candela/releases/latest\n"
        ;;
esac

printf "[Candela] Ground Control to Major Tom...\n"
printf "[Candela] Downloading $ARTIFACT for $OS/$ARCH\n"

TMP=$(mktemp -d)

# Clean up the temp directory once the script exits, for ANY reason
trap 'rm -rf "$TMP"' EXIT

$DOWNLOAD_CMD "https://github.com/lumen-fx/candela/releases/latest/download/$ARTIFACT.tar.gz" | tar -xz -C "$TMP"

if [ ! -f "$TMP/candela" ]; then
    # The github workflow packs the binary straight into an archive so something went very wrong here
    printf "[ERROR] Archive downloaded but binary not found inside. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

if [ ! -f "$TMP/candela-vm" ]; then
    # The release archive ships the VM-only runtime (candela-vm) alongside the
    # full compiler (candela) so a `.cdlb` can run without the compiler.
    printf "[ERROR] Archive downloaded but candela-vm not found inside. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

if [ ! -d "$TMP/libs/std" ]; then
    # The archive ships the standard library in libs/ next to the binary. `import
    # std::x` resolves relative to the installed binary, so this must be present.
    printf "[ERROR] Archive downloaded but the standard library (libs/std) is missing. Please file a bug report at https://github.com/lumen-fx/candela/issues\n"
fi

# Copy the whole archive, so the binary AND the libs/ tree (which holds the std
# library) land together in INSTALL_DIR. The binary resolves `import std::x`
# relative to its own location, so libs/ must sit beside it: INSTALL_DIR/candela
# and INSTALL_DIR/libs/std are the single source of truth for that lookup.
if cp -R "$TMP/." "$INSTALL_DIR" 2>/dev/null; then
    :
elif command -v sudo >/dev/null 2>&1; then
    sudo cp -R "$TMP/." "$INSTALL_DIR"
else
    printf "[ERROR] Cannot write to $INSTALL_DIR and sudo is not available. Re-run as root or install sudo.\n"
fi

if [ ! -d "$INSTALL_DIR/libs/std" ]; then
    printf "[ERROR] Standard library not installed at $INSTALL_DIR/libs/std. 'import std::x' will not resolve. Please re-run the installer.\n"
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

printf "[Candela] Installed $("$INSTALL_DIR/candela" --version) in $INSTALL_DIR/candela\n"
printf "[Candela] Installed candela-vm in $INSTALL_DIR/candela-vm\n"
printf "[Candela] Run 'candela' to get started.\n"
