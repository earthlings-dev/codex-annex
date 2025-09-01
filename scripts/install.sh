#!/usr/bin/env bash

# annex/scripts/install.sh

set -euo pipefail
bash scripts/bootstrap_external.sh
bash scripts/patch_codex_annex.sh
cargo install --path external/openai-codex/codex-cli \
  --features "annex-all" --force
echo "Try: codex mcp serve --stdio"
echo "Try: codex mcp serve --sse --port 8848"
echo "Try: codex mcp connect --server everything"
