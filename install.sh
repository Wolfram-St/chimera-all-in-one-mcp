#!/usr/bin/env bash
# ==============================================================================
# Chimera Universal Cross-Platform Installer (Linux & macOS)
# ==============================================================================
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Wolfram-St/chimera-all-in-one-mcp/main/install.sh | bash
# Or locally inside the repository:
#   ./install.sh
# ==============================================================================

set -e

BOLD='\033[1m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
RESET='\033[0m'

echo -e "${CYAN}${BOLD}"
echo "   ____ _     _                                "
echo "  / ___| |__ (_)_ __ ___   ___ _ __ __ _       "
echo " | |   | '_ \| | '_ \` _ \ / _ \ '__/ _\` |      "
echo " | |___| | | | | | | | | |  __/ | | (_| |      "
echo "  \____|_| |_|_|_| |_| |_|\___|_|  \__,_|      "
echo "  All-in-One Dynamic MCP Gateway & Skill Engine"
echo -e "${RESET}"

OS="$(uname -s)"
ARCH="$(uname -m)"
echo -e "${BOLD}Detected System:${RESET} ${OS} (${ARCH})"

# 1. Ensure Cargo / Rust is available
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}Rust / Cargo is not found in your PATH.${RESET}"
    echo "Installing Rust toolchain via rustup..."
    if command -v curl &> /dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    elif command -v wget &> /dev/null; then
        wget -qO- https://sh.rustup.rs | sh -s -- -y
    else
        echo -e "${RED}Error: Neither curl nor wget is available. Please install rustup manually.${RESET}"
        exit 1
    fi
    # Source cargo environment for this shell session
    if [ -f "$HOME/.cargo/env" ]; then
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi
fi

if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Error: Cargo is still not available after installation. Please restart your shell and re-run.${RESET}"
    exit 1
fi

# 2. Determine source repository
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
CHIMERA_DIR="$HOME/.chimera"
mkdir -p "$CHIMERA_DIR"

if [ -f "$SCRIPT_DIR/Cargo.toml" ] && [ -d "$SCRIPT_DIR/chimera-cli" ]; then
    SRC_DIR="$SCRIPT_DIR"
    echo -e "${GREEN}Building Chimera from local repository at ${SRC_DIR}...${RESET}"
else
    SRC_DIR="$CHIMERA_DIR/repo"
    if [ -d "$SRC_DIR/.git" ]; then
        echo -e "${CYAN}Updating existing Chimera source at ${SRC_DIR}...${RESET}"
        git -C "$SRC_DIR" pull --ff-only || true
    else
        echo -e "${CYAN}Cloning Chimera repository into ${SRC_DIR}...${RESET}"
        git clone https://github.com/Wolfram-St/chimera-all-in-one-mcp.git "$SRC_DIR"
    fi
fi

# 3. Compile release binaries
echo -e "${CYAN}Compiling Chimera release binaries...${RESET}"
cargo build --release --manifest-path "$SRC_DIR/Cargo.toml"

# 4. Install binaries into ~/.cargo/bin (standard Rust binary path)
INSTALL_DIR="$HOME/.cargo/bin"
mkdir -p "$INSTALL_DIR"

cp -f "$SRC_DIR/target/release/chimera-cli" "$INSTALL_DIR/chimera-cli"
cp -f "$SRC_DIR/target/release/chimera-proxy" "$INSTALL_DIR/chimera-proxy"
chmod +x "$INSTALL_DIR/chimera-cli" "$INSTALL_DIR/chimera-proxy"

echo -e "${GREEN}✓ Installed 'chimera-cli' and 'chimera-proxy' into ${INSTALL_DIR}${RESET}"

# Copy default registry seed if not present in ~/.chimera
if [ -f "$SRC_DIR/chimera_registry.json" ] && [ ! -f "$CHIMERA_DIR/chimera_registry.json" ]; then
    cp "$SRC_DIR/chimera_registry.json" "$CHIMERA_DIR/chimera_registry.json"
fi
if [ -f "$SRC_DIR/registry.db" ] && [ ! -f "$CHIMERA_DIR/registry.db" ]; then
    cp "$SRC_DIR/registry.db" "$CHIMERA_DIR/registry.db"
fi

# 5. Ensure INSTALL_DIR is in PATH across shell configs
PATH_EXPORT='export PATH="$HOME/.cargo/bin:$PATH"'
add_to_profile() {
    local file="$1"
    if [ -f "$file" ]; then
        if ! grep -q '\.cargo/bin' "$file"; then
            echo -e "\n# Chimera CLI" >> "$file"
            echo "$PATH_EXPORT" >> "$file"
            echo -e "${CYAN}Added ~/.cargo/bin to ${file}${RESET}"
        fi
    fi
}

add_to_profile "$HOME/.bashrc"
add_to_profile "$HOME/.zshrc"
add_to_profile "$HOME/.profile"
add_to_profile "$HOME/.bash_profile"

export PATH="$INSTALL_DIR:$PATH"

# 6. Run automated harness setup for all detected AI harnesses
echo -e "\n${CYAN}Configuring installed AI harnesses (Cursor, Claude Desktop, Claude Code, Windsurf, Antigravity)...${RESET}"
"$INSTALL_DIR/chimera-cli" setup --all-clients || true

# 7. Print Completion Banner
echo -e "\n${GREEN}${BOLD}========================================================================${RESET}"
echo -e "${GREEN}${BOLD}✓ Chimera successfully installed and configured across all harnesses!${RESET}"
echo -e "${GREEN}${BOLD}========================================================================${RESET}"
echo -e "Quick Start Commands:"
echo -e "  ${BOLD}chimera-cli list${RESET}                 View installed and active MCP servers"
echo -e "  ${BOLD}chimera-cli add <package|url>${RESET}    Add zero-install MCP (npx:..., uvx:..., git)"
echo -e "  ${BOLD}chimera-cli cache list${RESET}           Inspect cache size across your drives"
echo -e "  ${BOLD}chimera-cli cache clean${RESET}          Reclaim disk space from orphaned repos"
echo -e "  ${BOLD}chimera-cli setup --all-clients${RESET} Wire Chimera into all agent clients"
echo -e "========================================================================\n"
