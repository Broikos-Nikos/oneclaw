#!/usr/bin/env bash
set -euo pipefail

# OneClaw Installer
# Usage: curl -sSL https://raw.githubusercontent.com/Broikos-Nikos/oneclaw/main/install.sh | bash

REPO="Broikos-Nikos/oneclaw"
BRANCH="main"

echo ""
echo "OneClaw Installer"
echo "  Multi-agent AI assistant with router architecture"
echo ""

# ─── Detect platform ─────────────────────────────────────────────────────

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)   PLATFORM="linux" ;;
  Darwin)  PLATFORM="macos" ;;
  *)       echo "❌ Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_TAG="x86_64" ;;
  aarch64|arm64) ARCH_TAG="aarch64" ;;
  *)             echo "❌ Unsupported architecture: $ARCH"; exit 1 ;;
esac

echo "  Platform: $PLATFORM ($ARCH_TAG)"
echo ""

# ─── Ensure dependencies ─────────────────────────────────────────────────

have_cmd() { command -v "$1" >/dev/null 2>&1; }

# Install Rust if not present
if ! have_cmd cargo; then
  echo "📦 Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  source "$HOME/.cargo/env"
  echo "✅ Rust installed"
else
  echo "✅ Rust already installed ($(rustc --version))"
fi

# Ensure git
if ! have_cmd git; then
  echo "📦 Installing git..."
  if have_cmd apt-get; then
    sudo apt-get update -qq && sudo apt-get install -y git
  elif have_cmd dnf; then
    sudo dnf install -y git
  elif have_cmd pacman; then
    sudo pacman -Sy --noconfirm git
  elif have_cmd brew; then
    brew install git
  else
    echo "❌ Please install git manually and retry."
    exit 1
  fi
fi

# ─── Clone and build ─────────────────────────────────────────────────────

INSTALL_DIR="$HOME/.oneclaw/src"

if [ -d "$INSTALL_DIR" ]; then
  echo "📦 Updating existing source..."
  cd "$INSTALL_DIR"
  git pull --ff-only origin "$BRANCH" 2>/dev/null || true
else
  echo "📦 Cloning OneClaw..."
  git clone --depth 1 --branch "$BRANCH" "https://github.com/$REPO.git" "$INSTALL_DIR"
  cd "$INSTALL_DIR"
fi

echo "🔨 Building OneClaw (release mode)..."
cargo build --release

# ─── Install binary ──────────────────────────────────────────────────────

CARGO_BIN="$HOME/.cargo/bin"
mkdir -p "$CARGO_BIN"
cp "target/release/oneclaw" "$CARGO_BIN/oneclaw"
chmod +x "$CARGO_BIN/oneclaw"

# Ensure cargo bin is in PATH
if ! echo "$PATH" | grep -q "$CARGO_BIN"; then
  SHELL_RC=""
  if [ -f "$HOME/.bashrc" ]; then
    SHELL_RC="$HOME/.bashrc"
  elif [ -f "$HOME/.zshrc" ]; then
    SHELL_RC="$HOME/.zshrc"
  fi
  if [ -n "$SHELL_RC" ]; then
    echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> "$SHELL_RC"
    echo "  Added $CARGO_BIN to PATH in $SHELL_RC"
  fi
  export PATH="$CARGO_BIN:$PATH"
fi

# ─── Initial setup ───────────────────────────────────────────────────────

echo ""
echo "✅ OneClaw installed to: $CARGO_BIN/oneclaw"
echo ""

# Run onboard with defaults if no config exists
ONECLAW_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/oneclaw"
if [ ! -f "$ONECLAW_CONFIG_DIR/config.toml" ]; then
  echo "🔧 Running initial setup..."
  echo ""
  oneclaw onboard
else
  echo "Existing config found. Skipping setup."
  echo ""
  echo "Usage:"
  echo "  oneclaw agent                         # interactive chat"
  echo "  oneclaw agent -m \"your message\"        # single-shot"
  echo "  oneclaw create-agent developer         # add a sub-agent"
  echo "  oneclaw list-agents                    # see all agents"
fi

echo ""
echo "🦀 Done! Run 'oneclaw agent' to start."
echo ""
