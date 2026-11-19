#!/bin/bash
# Puts the settings-corsair helper in bin/: the released build matching this
# version when there is one, otherwise built from source (needs Rust).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p bin

version=$(sed -n 's/^version = "\(.*\)"/\1/p' extension.toml)
asset="settings-corsair-$(uname -m)"
url="https://github.com/design-nexus/nyx/releases/download/ext-corsair-v$version/$asset"
if curl -fsSL --max-time 60 -o bin/.settings-corsair.tmp "$url" 2>/dev/null; then
  chmod +x bin/.settings-corsair.tmp
  mv bin/.settings-corsair.tmp bin/settings-corsair
  exit 0
fi
rm -f bin/.settings-corsair.tmp

if ! command -v cargo >/dev/null; then
  echo "Building the Corsair extension requires Rust and Cargo on PATH." >&2
  exit 1
fi
cargo build --release --locked --quiet
install -m 755 target/release/settings-corsair bin/settings-corsair
