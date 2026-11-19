#!/bin/bash
# Puts the settings-asus helper in bin/: the released build when there is one,
# otherwise built from source (needs Rust).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p bin

# The release matching this checkout's version (tagged ext-asus-v<version> in nyx).
version=$(sed -n 's/^version = "\(.*\)"/\1/p' extension.toml)
asset="settings-asus-$(uname -m)"
url="https://github.com/design-nexus/nyx/releases/download/ext-asus-v$version/$asset"
if curl -fsSL --max-time 60 -o bin/.settings-asus.tmp "$url" 2>/dev/null; then
  chmod +x bin/.settings-asus.tmp
  mv bin/.settings-asus.tmp bin/settings-asus
  exit 0
fi
rm -f bin/.settings-asus.tmp

if ! command -v cargo >/dev/null; then
  echo "Building the ASUS extension requires Rust and Cargo on PATH." >&2
  exit 1
fi
cargo build --release --locked --quiet
install -m 755 target/release/settings-asus bin/settings-asus
