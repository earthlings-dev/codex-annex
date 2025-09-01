# annex/Makefile

.PHONY: init patch install update build

init:
	@bash scripts/bootstrap_external.sh
	# also init the codex submodule (must already be added in your repo)
	@git submodule update --init --recursive --jobs 4
	# Make sure sparse includes codex-rs & codex-cli
	@git -C external/openai-codex sparse-checkout init --cone || true
	@git -C external/openai-codex sparse-checkout set codex-rs codex-cli || true
	@echo "✔ submodules ready"

patch:
	@bash scripts/patch_codex_annex.sh

build: init patch
	cargo build --workspace

install: init patch
	# Install only codex-owned bins; no annex binary.
	cargo install --path external/openai-codex/codex-cli \
		--features "annex,annex-mcp,annex-mcp-sse,annex-mcp-stream" --force
	@echo "✔ installed: codex (existing bin) and codex-mcp (new bin)"

update:
	@git submodule sync --recursive
	@git submodule update --remote --recursive --jobs 4
	@git add external/* || true
	@git commit -m "chore: bump external submodules" || true