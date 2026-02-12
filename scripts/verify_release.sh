#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "[verify_release] cargo fmt --check"
cargo fmt --all --check

if [[ -x "scripts/forbid_string_maps.sh" ]]; then
  echo "[verify_release] forbid_string_maps"
  scripts/forbid_string_maps.sh
fi

echo "[verify_release] cargo check (release)"
cargo check --workspace --all-targets --all-features --release

echo "[verify_release] cargo clippy (release, -D warnings)"
cargo clippy --workspace --all-targets --all-features --release -- -D warnings

echo "[verify_release] cargo test (release)"
cargo test --workspace --all-features --release

echo "[verify_release] cargo doc"
cargo doc --workspace --all-features --no-deps

echo "[verify_release] OK"
