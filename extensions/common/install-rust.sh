#!/bin/bash
# install-rust.sh ID NAME: puts an extension's settings-ID helper in its bin/,
# the released build matching its version (ext-ID-v<version> in nyx) when
# there is one, otherwise built from source (needs Rust).
set -euo pipefail
id=$1 name=$2
dir="$(cd "$(dirname "$0")/../$id" && pwd)"
cd "$dir"
mkdir -p bin
version=$(sed -n 's/^version = "\(.*\)"/\1/p' extension.toml)
asset="settings-$id-$(uname -m)"
url="https://github.com/design-nexus/nyx/releases/download/ext-$id-v$version/$asset"
if curl -fsSL --max-time 60 -o "bin/.settings-$id.tmp" "$url" 2>/dev/null; then
  chmod +x "bin/.settings-$id.tmp"
  mv "bin/.settings-$id.tmp" "bin/settings-$id"
  exit 0
fi
rm -f "bin/.settings-$id.tmp"
if ! command -v cargo >/dev/null; then
  echo "Building the $name extension requires Rust and Cargo on PATH." >&2
  exit 1
fi
cargo build --release --quiet
install -m 755 "target/release/settings-$id" "bin/settings-$id"
