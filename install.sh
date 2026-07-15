#!/bin/sh
# GX Language Installer
# Usage: curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

set -e

# Resolved from GitHub's "latest release" API at install time (see
# detect_latest_version below) unless the caller already set GX_VERSION —
# e.g. `GX_VERSION=0.6.1 sh install.sh` to pin a specific release. This
# used to be a hardcoded literal here, which meant every install after
# the version it was hardcoded to went stale: a fresh machine following
# this exact script would silently get an old release with none of the
# fixes documented in the CHANGELOG since, several versions behind
# whatever `gx --version` on the maintainer's own machine reported.
GX_VERSION="${GX_VERSION:-}"
GX_REPO="elgrhy/gx"
GX_HOME="${GX_HOME:-$HOME/.gx}"
GX_BIN="$GX_HOME/bin"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()    { printf "${BLUE}info${NC}  %s\n" "$1"; }
success() { printf "${GREEN}ok${NC}    %s\n" "$1"; }
warn()    { printf "${YELLOW}warn${NC}  %s\n" "$1"; }
error()   { printf "${RED}error${NC} %s\n" "$1" >&2; exit 1; }

# ── Detect OS and architecture ────────────────────────────────────────────────

detect_target() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    # ARCHIVE names must match release.yml's `matrix.archive` exactly —
    # see build_from_source's fallback for the one target (Linux armv7)
    # release.yml's build matrix doesn't publish a pre-built binary for
    # at all, where ARCHIVE is deliberately left empty.
    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-gnu"; ARCHIVE="gx-linux-x64.tar.gz" ;;
                aarch64) TARGET="aarch64-unknown-linux-gnu"; ARCHIVE="gx-linux-arm64.tar.gz" ;;
                armv7l)  TARGET="armv7-unknown-linux-gnueabihf"; ARCHIVE="" ;;
                *)       error "Unsupported Linux architecture: $ARCH" ;;
            esac
            EXT=""
            ;;
        darwin)
            case "$ARCH" in
                x86_64) TARGET="x86_64-apple-darwin"; ARCHIVE="gx-macos-x64.tar.gz" ;;
                arm64)  TARGET="aarch64-apple-darwin"; ARCHIVE="gx-macos-arm64.tar.gz" ;;
                *)      error "Unsupported macOS architecture: $ARCH" ;;
            esac
            EXT=""
            ;;
        mingw*|cygwin*|msys*)
            TARGET="x86_64-pc-windows-msvc"
            ARCHIVE="gx-windows-x64.zip"
            EXT=".exe"
            ;;
        *)
            error "Unsupported operating system: $OS"
            ;;
    esac
}

# ── Resolve version ───────────────────────────────────────────────────────────

detect_latest_version() {
    info "Looking up the latest GX release..."

    API_URL="https://api.github.com/repos/$GX_REPO/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        LATEST_TAG=$(curl -sSfL "$API_URL" 2>/dev/null | grep '"tag_name"' | head -n 1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        LATEST_TAG=$(wget -qO- "$API_URL" 2>/dev/null | grep '"tag_name"' | head -n 1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    else
        error "Neither curl nor wget found. Please install one and try again."
    fi

    if [ -z "$LATEST_TAG" ]; then
        error "Could not determine the latest GX release from GitHub (network issue, or the API response shape changed). Set GX_VERSION explicitly to install a specific release, e.g.: GX_VERSION=0.6.1 sh install.sh"
    fi

    # Release tags are "v0.7.0" — strip the leading "v".
    GX_VERSION="${LATEST_TAG#v}"
    info "Latest GX release: v$GX_VERSION"
}

# ── Download ──────────────────────────────────────────────────────────────────

# Downloads and unpacks the pre-built release archive for this target.
# Returns non-zero (rather than exiting) on *any* failure — missing
# ARCHIVE, a failed download, or a failed extraction — so `main`'s `if !
# download_gx; then build_from_source; fi` can actually fall back, the
# way it always should have. The previous version's last statement was
# always a `success` print (which itself always exits 0), so the
# function's own exit status could never reflect a failed curl/wget
# call underneath it — every download failure silently fell through to
# `chmod`/`success` on a file that was never written, and the fallback
# to `build_from_source` never ran.
#
# The download URL itself was also simply wrong: it requested
# `gx-$TARGET$EXT` (e.g. `gx-aarch64-apple-darwin`), a literal binary
# file that release.yml has never published under any name — every
# real release asset is a `.tar.gz`/`.zip` archive named
# `gx-<os>-<arch>.<ext>` (see `matrix.archive` in
# .github/workflows/release.yml), containing the binary inside it. This
# was never version-dependent — every install of every past release hit
# this, always fell through to build_from_source (silently, since the
# failure was swallowed) rather than actually installing the pre-built
# binary it claimed to.
download_gx() {
    if [ -z "$ARCHIVE" ]; then
        # No pre-built binary is published for this target.
        return 1
    fi

    DOWNLOAD_URL="https://github.com/$GX_REPO/releases/download/v$GX_VERSION/$ARCHIVE"
    info "Downloading GX v$GX_VERSION for $TARGET..."

    mkdir -p "$GX_BIN"
    TMP_ARCHIVE=$(mktemp)

    if command -v curl >/dev/null 2>&1; then
        if ! curl -sSfL "$DOWNLOAD_URL" -o "$TMP_ARCHIVE"; then
            rm -f "$TMP_ARCHIVE"
            return 1
        fi
    elif command -v wget >/dev/null 2>&1; then
        if ! wget -q "$DOWNLOAD_URL" -O "$TMP_ARCHIVE"; then
            rm -f "$TMP_ARCHIVE"
            return 1
        fi
    else
        error "Neither curl nor wget found. Please install one and try again."
    fi

    case "$ARCHIVE" in
        *.tar.gz)
            if ! tar xzf "$TMP_ARCHIVE" -C "$GX_BIN" "gx$EXT" 2>/dev/null; then
                rm -f "$TMP_ARCHIVE"
                return 1
            fi
            ;;
        *.zip)
            if ! command -v unzip >/dev/null 2>&1; then
                warn "unzip not found — cannot extract the pre-built Windows archive, falling back to a source build."
                rm -f "$TMP_ARCHIVE"
                return 1
            fi
            if ! unzip -qo "$TMP_ARCHIVE" -d "$GX_BIN" 2>/dev/null; then
                rm -f "$TMP_ARCHIVE"
                return 1
            fi
            ;;
    esac
    rm -f "$TMP_ARCHIVE"

    if ! chmod +x "$GX_BIN/gx$EXT"; then
        return 1
    fi
    success "Downloaded gx to $GX_BIN/gx$EXT"
}

# ── Build from source (fallback) ──────────────────────────────────────────────

build_from_source() {
    warn "No pre-built binary found for $TARGET — building from source..."

    if ! command -v cargo >/dev/null 2>&1; then
        error "Rust/Cargo not found. Install from https://rustup.rs then try again."
    fi

    TMPDIR=$(mktemp -d)
    info "Cloning GX repository..."

    if command -v git >/dev/null 2>&1; then
        git clone --depth 1 "https://github.com/$GX_REPO.git" "$TMPDIR/gx" >/dev/null 2>&1
    else
        error "git not found. Please install git."
    fi

    info "Building GX (this may take a minute)..."
    cd "$TMPDIR/gx"
    cargo build --release --quiet

    mkdir -p "$GX_BIN"
    cp "target/release/gx$EXT" "$GX_BIN/gx$EXT"
    chmod +x "$GX_BIN/gx$EXT"

    cd /
    rm -rf "$TMPDIR"
    success "Built and installed gx to $GX_BIN/gx$EXT"
}

# ── Setup PATH ────────────────────────────────────────────────────────────────

setup_path() {
    SHELL_NAME=$(basename "$SHELL")
    PATH_LINE="export PATH=\"\$PATH:$GX_BIN\""

    add_to_file() {
        FILE="$1"
        if [ -f "$FILE" ] && ! grep -q "GX_HOME" "$FILE" 2>/dev/null; then
            printf "\n# GX Language\n%s\n" "$PATH_LINE" >> "$FILE"
            success "Added GX to PATH in $FILE"
        fi
    }

    case "$SHELL_NAME" in
        zsh)  add_to_file "$HOME/.zshrc" ;;
        bash) add_to_file "$HOME/.bashrc"; add_to_file "$HOME/.bash_profile" ;;
        fish)
            FISH_CONFIG="$HOME/.config/fish/config.fish"
            mkdir -p "$(dirname "$FISH_CONFIG")"
            if ! grep -q "GX_HOME" "$FISH_CONFIG" 2>/dev/null; then
                printf "\n# GX Language\nfish_add_path %s\n" "$GX_BIN" >> "$FISH_CONFIG"
                success "Added GX to PATH in $FISH_CONFIG"
            fi
            ;;
        *)
            add_to_file "$HOME/.profile"
            ;;
    esac
}

# ── Verify installation ───────────────────────────────────────────────────────

verify() {
    if "$GX_BIN/gx$EXT" version >/dev/null 2>&1; then
        VERSION_OUT=$("$GX_BIN/gx$EXT" version)
        success "Installed: $VERSION_OUT"
    else
        error "Installation failed — gx binary does not run correctly"
    fi
}

# ── Main ──────────────────────────────────────────────────────────────────────

main() {
    printf "\n${BOLD}GX Language Installer${NC}\n"
    printf "Brain-first programming language\n\n"

    detect_target

    if [ -z "$GX_VERSION" ]; then
        detect_latest_version
    else
        info "Using pinned GX_VERSION=$GX_VERSION"
    fi

    # Try pre-built binary first, fall back to source build
    if ! download_gx 2>/dev/null; then
        build_from_source
    fi

    setup_path
    verify

    printf "\n${BOLD}${GREEN}GX installed successfully!${NC}\n\n"
    printf "Restart your terminal or run:\n"
    printf "  ${BOLD}export PATH=\"\$PATH:$GX_BIN\"${NC}\n\n"
    printf "Then try:\n"
    printf "  ${BOLD}gx help${NC}\n"
    printf "  ${BOLD}gx init my-first-agent${NC}\n"
    printf "  ${BOLD}cd my-first-agent && gx run main.gx${NC}\n\n"
}

main "$@"
