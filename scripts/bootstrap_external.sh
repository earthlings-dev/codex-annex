#!/usr/bin/env bash

# annex/scripts/bootstrap_external.sh

set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

add() {
  local url="$1" dest="$2" branch="${3:-main}"
  if [ ! -d "$root/$dest/.git" ]; then
    git submodule add -b "$branch" --depth 1 "$url" "$dest"
  fi
}

mkdir -p "$root/external"

# MCP Rust SDK (official)
add "https://github.com/modelcontextprotocol/rust-sdk" "external/mcp-rust-sdk"

# Zed Agent Client Protocol (ACP)
add "https://github.com/zed-industries/agent-client-protocol" "external/agent-client-protocol"

# LangChain Agent Protocol (OpenAPI + reference server)
add "https://github.com/langchain-ai/agent-protocol" "external/agent-protocol"

# A2A (Agent2Agent)
add "https://github.com/a2aproject/A2A" "external/a2a"

git submodule update --init --recursive

# Keep checkouts lean with sparse
git -C "$root/external/mcp-rust-sdk" sparse-checkout init --cone || true
git -C "$root/external/mcp-rust-sdk" sparse-checkout set crates README.md || true

git -C "$root/external/agent-client-protocol" sparse-checkout init --cone || true
git -C "$root/external/agent-client-protocol" sparse-checkout set rust schema Cargo.toml README.md || true

git -C "$root/external/agent-protocol" sparse-checkout init --cone || true
git -C "$root/external/agent-protocol" sparse-checkout set openapi.json api.html server README.md || true

git -C "$root/external/a2a" sparse-checkout init --cone || true
git -C "$root/external/a2a" sparse-checkout set specification README.md || true

echo "✔ external submodules added and sparsified"