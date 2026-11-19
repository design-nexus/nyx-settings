#!/bin/bash
# Puts the settings-logitech helper in bin/: the released build matching this
# version when there is one, otherwise built from source (needs Rust).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p bin

version=$(sed -n 's/^version = "\(.*\)"/\1/p' extension.toml)
asset="settings-logitech-$(uname -m)"
url="https://github.com/design-nexus/nyx/releases/download/ext-logitech-v$version/$asset"
if curl -fsSL --max-time 60 -o bin/.settings-logitech.tmp "$url" 2>/dev/null; then
  chmod +x bin/.settings-logitech.tmp
  mv bin/.settings-logitech.tmp bin/settings-logitech
  exit 0
fi
rm -f bin/.settings-logitech.tmp

if ! command -v cargo >/dev/null; then
  echo "Building the Logitech extension requires Rust and Cargo on PATH." >&2
  exit 1
fi
cargo build --release --locked --quiet
install -m 755 target/release/settings-logitech bin/settings-logitech
