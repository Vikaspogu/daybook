#!/usr/bin/env bash
# Daybook - cross-runtime work journal
# Installation Script
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Vikaspogu/daybook/main/scripts/install.sh | bash
#
# Options:
#   DAYBOOK_VERSION - Install specific version (default: latest)
#   DAYBOOK_INSTALL_DIR - Installation directory (default: ~/.local/bin)
#   GITHUB_TOKEN - GitHub token to bypass rate limiting

set -euo pipefail

# Configuration
REPO="Vikaspogu/daybook"
BINARY_NAME="daybook"
DEFAULT_INSTALL_DIR="${HOME}/.local/bin"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Detect OS and Architecture
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)  os="linux" ;;
        Darwin*) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) error "Unsupported operating system: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) error "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "${os}-${arch}"
}

# Curl wrapper with optional GitHub token auth
curl_github() {
    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
        curl -H "Authorization: token ${GITHUB_TOKEN}" "$@"
    else
        curl "$@"
    fi
}

# Show rate limit hint
show_rate_limit_hint() {
    echo ""
    warn "GitHub API rate limit exceeded"
    echo ""
    echo "To bypass rate limiting, set GITHUB_TOKEN environment variable:"
    echo ""
    echo "  GITHUB_TOKEN=your_token curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | bash"
    echo ""
    echo "You can create a token at: https://github.com/settings/tokens"
    echo ""
}

# Get latest release version (outputs __RATE_LIMITED__ on rate limit)
get_latest_version() {
    local api_url="https://api.github.com/repos/${REPO}/releases/latest"
    local tmp_file
    tmp_file=$(mktemp)
    local http_code
    http_code=$(curl_github -sL --connect-timeout 10 --max-time 30 -w "%{http_code}" -o "$tmp_file" "$api_url") || true

    if [[ "$http_code" == "403" || "$http_code" == "429" ]]; then
        rm -f "$tmp_file"
        echo "__RATE_LIMITED__"
        return
    fi

    if [[ "$http_code" != "200" ]]; then
        rm -f "$tmp_file"
        return
    fi

    local version
    version=$(grep '"tag_name"' "$tmp_file" 2>/dev/null | sed -E 's/.*"([^"]+)".*/\1/' || true)
    rm -f "$tmp_file"
    echo "$version"
}

# Download and install binary
install_daybook() {
    local platform="$1"
    local version="$2"
    local install_dir="$3"

    local artifact_name="${BINARY_NAME}-${platform}"
    local target_name="${BINARY_NAME}"
    if [[ "$platform" == "windows"* ]]; then
        artifact_name="${artifact_name}.exe"
        target_name="${target_name}.exe"
    fi

    local download_url="https://github.com/${REPO}/releases/download/${version}/${artifact_name}"

    info "Downloading Daybook ${version} for ${platform}..."
    info "URL: ${download_url}"

    # Create temp directory
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap "rm -rf ${tmp_dir}" EXIT

    local tmp_file="${tmp_dir}/${BINARY_NAME}"

    # Download binary
    local http_code
    http_code=$(curl_github -sL --connect-timeout 10 --max-time 120 -w "%{http_code}" -o "$tmp_file" "$download_url") || true

    if [[ "$http_code" == "403" || "$http_code" == "429" ]]; then
        show_rate_limit_hint
        exit 1
    fi

    if [[ "$http_code" != "200" ]]; then
        error "Failed to download binary from ${download_url} (HTTP $http_code)"
    fi

    # Make executable
    chmod +x "$tmp_file"

    # Create install directory if not exists
    mkdir -p "$install_dir"

    # Move to install directory
    local target="${install_dir}/${target_name}"
    mv "$tmp_file" "$target"
    INSTALLED_PATH="${target}"

    success "Installed Daybook to ${target}"
}

# Run post-install setup
post_install() {
    local install_dir="$1"

    echo ""
    success "Daybook has been installed successfully!"
    echo ""

    # Check if directory is in PATH
    if [[ ":$PATH:" != *":${install_dir}:"* ]]; then
        warn "${install_dir} is not in your PATH"
        echo ""
        echo "Add the following to your shell configuration file (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"\$PATH:${install_dir}\""
        echo ""

        # Temporarily add to PATH for this session
        export PATH="$PATH:${install_dir}"
        info "Temporarily added ${install_dir} to PATH for this session"
        echo ""
    fi

    # Interactive setup only when stdin is a tty (curl | bash is not)
    if [[ -t 0 ]]; then
        info "Running initial setup..."
        "${INSTALLED_PATH}" init
    else
        info "Run '${INSTALLED_PATH} init' to set up daybook on this machine."
    fi

    echo ""
    echo "For more information, visit:"
    echo "  https://github.com/${REPO}"
}

main() {
    echo ""
    echo "  ╔═══════════════════════════════════════════════════╗"
    echo "  ║          Daybook Installation Script              ║"
    echo "  ║    Cross-runtime work journal for agent sessions  ║"
    echo "  ╚═══════════════════════════════════════════════════╝"
    echo ""

    # Get platform
    local platform
    platform=$(detect_platform)
    info "Detected platform: ${platform}"

    # Get version
    local version="${DAYBOOK_VERSION:-}"
    if [[ -z "$version" ]]; then
        info "Fetching latest version..."
        version=$(get_latest_version) || true
        if [[ "$version" == "__RATE_LIMITED__" ]]; then
            show_rate_limit_hint
            exit 1
        fi
        if [[ -z "$version" ]]; then
            error "Failed to get latest version. Please specify DAYBOOK_VERSION."
        fi
    fi
    info "Version: ${version}"

    # Get install directory
    local install_dir="${DAYBOOK_INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
    info "Install directory: ${install_dir}"

    # Install
    install_daybook "$platform" "$version" "$install_dir"

    # Post-install instructions (includes PATH check)
    post_install "$install_dir"
}

main "$@"
