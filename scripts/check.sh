#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo "=== Checking firmware crate ==="
cd "$PROJECT_ROOT/firmware"
cargo fmt --check
echo "Building for stm32f411..."
cargo build --release --features stm32f411
echo "Building for stm32f412..."
cargo build --release --features stm32f412

echo "=== Checking control crate ==="
cd "$PROJECT_ROOT/control"
cargo fmt --check
wasm-pack build --target web --out-dir www/pkg

echo "=== All checks passed ==="
