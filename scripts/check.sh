#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

echo "=== Checking firmware crate ==="
cd "$PROJECT_ROOT/firmware"
cargo fmt --check
cargo build --release

echo "=== Checking control crate ==="
cd "$PROJECT_ROOT/control"
cargo fmt --check
wasm-pack build --target web --out-dir www/pkg

echo "=== All checks passed ==="
