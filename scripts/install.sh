#!/usr/bin/env bash

# annex/scripts/install.sh

set -euo pipefail
bash scripts/bootstrap_external.sh
bash scripts/patch_codex_annex.sh
cargo install --path external/openai-codex/codex-cli \
  --features "annex,annex-mcp,annex-mcp-sse,annex-mcp-stream" --force
echo "Try: codex-mcp --mode stdio    # run server over stdio"
echo "Try: codex-mcp --mode sse --addr 127.0.0.1:8848"
echo "Try: codex-mcp --mode streamable-http --addr 127.0.0.1:8849 --http_path /mcp"