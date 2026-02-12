#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

targets=(
  "wavyte/src/eval"
  "wavyte/src/compile"
  "wavyte/src/render"
)

pattern='(HashMap|BTreeMap)<[[:space:]]*String'

hits=0
for dir in "${targets[@]}"; do
  if [[ -d "$dir" ]]; then
    if rg -n -S "$pattern" "$dir" -g '*.rs' >/dev/null; then
      echo "[forbid_string_maps] FAIL: string-keyed maps found under $dir"
      rg -n -S "$pattern" "$dir" -g '*.rs' || true
      hits=1
    fi
  fi
done

if [[ "$hits" -ne 0 ]]; then
  exit 1
fi

echo "[forbid_string_maps] OK"
