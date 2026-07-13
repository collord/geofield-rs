#!/usr/bin/env bash
# Rebuild the web3 bundle before an e2e / Playwright run so tests never hit a stale
# dist/. This project compiles Rust → WASM → JS; `vite preview` serves the built dist/,
# so an edit to src/** or crates/** is invisible until rebuilt. Called from a PreToolUse
# hook on `playwright test` (see .claude/settings.json).
#
# - If any crates/ Rust source is newer than the built engine wasm, rebuild wasm first
#   (slow: cargo + wasm-pack). Otherwise skip it.
# - Always rebuild the JS bundle (fast: tsc + vite, ~8s).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
web3="$repo_root/web3"
engine_wasm="$web3/src/wasm/geofield_bg.wasm"

cd "$web3"

# Does any crates/ source out-date the built engine wasm? If the wasm is missing, force it.
needs_wasm=0
if [[ ! -f "$engine_wasm" ]]; then
  needs_wasm=1
elif [[ -n "$(find "$repo_root/crates" -name '*.rs' -newer "$engine_wasm" -print -quit 2>/dev/null)" ]]; then
  needs_wasm=1
fi

if [[ "$needs_wasm" == "1" ]]; then
  echo "[prebuild-for-e2e] crates/ changed since last wasm build — rebuilding wasm (slow)…" >&2
  npm run wasm
fi

echo "[prebuild-for-e2e] building JS bundle…" >&2
npm run build
