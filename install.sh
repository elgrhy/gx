#!/bin/sh
# GX Language Installer
# Usage: curl -sSf https://raw.githubusercontent.com/elgrhy/gx/main/install.sh | sh

set -e

GX_VERSION="0.1.0"
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

    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
                aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
                armv7l)  TARGET="armv7-unknown-linux-gnueabihf" ;;
                *)       error "Unsupported Linux architecture: $ARCH" ;;
            esac
            EXT=""
            ;;
        darwin)
            case "$ARCH" in
                x86_64) TARGET="x86_64-apple-darwin" ;;
                arm64)  TARGET="aarch64-apple-darwin" ;;
                *)      error "Unsupported macOS architecture: $ARCH" ;;
            esac
            EXT=""
            ;;
        mingw*|cygwin*|msys*)
            TARGET="x86_64-pc-windows-msvc"
            EXT=".exe"
            ;;
        *)
            error "Unsupported operating system: $OS"
            ;;
    esac
}

# ── Download ──────────────────────────────────────────────────────────────────

download_gx() {
    DOWNLOAD_URL="https://github.com/$GX_REPO/releases/download/v$GX_VERSION/gx-$TARGET$EXT"

    info "Downloading GX v$GX_VERSION for $TARGET..."

    mkdir -p "$GX_BIN"

    if command -v curl >/dev/null 2>&1; then
        curl -sSfL "$DOWNLOAD_URL" -o "$GX_BIN/gx$EXT"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$DOWNLOAD_URL" -O "$GX_BIN/gx$EXT"
    else
        error "Neither curl nor wget found. Please install one and try again."
    fi

    chmod +x "$GX_BIN/gx$EXT"
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
