File: `annex/AGENTS.md`

# Annex Agents & Protocol Surfaces

**Status:** implementation-ready  
**Audience:** contributors & integrators building on top of `annex` inside `codex-rs`  
**Scope:** agent runtime semantics, protocol surfaces (MCP/ACP/A2A/AGP), configuration, execution model, and operational guidance.

---

## Why this document exists

This file is the canonical, high‑fidelity reference for how **agents** work inside the `annex` feature set. It consolidates architectural intent and the operational contract that the runtime and configuration must uphold.

**Authoritative source order:** **conversation → repo working tree → “Project Files” pane**. If there is a conflict, the guidance in this conversation thread supersedes any stale project files.

---

## Quick Start

> Assumptions & Versions
>
> - OS: macOS 15+ / Ubuntu 22.04+  
> - Toolchain: Rust **stable** (≥ 1.98), Bun **1.2.21+**, Python **3.13+**, Bash **5+**, GNU Make **4+**  
> - No secrets in code. Use env vars: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, etc.

1) **Bootstrap, patch, build, install**:

```bash
make init       # bootstrap externals, set sparse submodules, init codex
make patch      # apply codex-rs/cli feature & file patches for annex
make build      # cargo build --workspace
make install    # install codex with annex* features enabled (no separate annex bin)
```

2) **Create minimal `.codex/` workspace** (TOML config + JSON data):

```bash
mkdir -p .codex/{hooks,slash,tasks,todos,sessions}
```

**`.codex/config.toml`** (model routing; source of truth):

```toml
[models.default]
name = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[models.overrides.title]
name = "claude-3-5-haiku"
base_url = "https://api.anthropic.com"
api_token_env = "ANTHROPIC_API_KEY"

[models.overrides.session_name]
name = "gpt-4o-mini"

[models.overrides.compact]
name = "gemini-1.5-flash"
base_url = "https://generativelanguage.googleapis.com"
api_key_env = "GOOGLE_API_KEY"

[models.overrides.meta_prompt]
name = "claude-3.7-sonnet"
base_url = "https://api.anthropic.com"
api_token_env = "ANTHROPIC_API_KEY"

[models.overrides.task_status]
name = "gpt-4o-mini"

[models.profiles.fast]
name = "gpt-4o-mini"

[models.profiles.heavy]
name = "claude-3.7-sonnet"
base_url = "https://api.anthropic.com"
api_token_env = "ANTHROPIC_API_KEY"

[models.profiles.google]
name = "gemini-1.5-pro"
base_url = "https://generativelanguage.googleapis.com"
api_key_env = "GOOGLE_API_KEY"

[models.profiles.anthropic]
name = "claude-3.7-sonnet"
base_url = "https://api.anthropic.com"
api_token_env = "ANTHROPIC_API_KEY"
```

MCP servers live under `[mcp.servers.*]` in `.codex/config.toml`.

**`.codex/hooks/*.toml`** (example audit + status summarization hooks):

```toml
[[rule]]
name = "audit-log"
enabled = true
deny_on_fail = false
when = ["post_exec", "task_end"]

  [[rule.actions]]
  kind = "plugin"
  handler = "audit_log"

[[rule]]
name = "summarize-task"
enabled = true
deny_on_fail = false
when = ["task_end"]

  [[rule.actions]]
  kind = "prompt"
  model_profile = "heavy"
  instruction = "Generate a one-line status that explains what the task achieved and any blockers."
```

**`.codex/slash/*.toml`** (alias & macro; MCP mgmt in workspace scope):

```toml
[alias]
todo = "/todo $ARGS"

[[macro]]
name = "quick-title"
lines = [
  "/config-set models.overrides.title.name gpt-4o-mini",
  "/run title $ARGS"
]
```

**TaskSet spec example** — `.codex/tasks/2025-09-01/SESSION-UUID/set-01.json` (JSON):

```json
{
  "sets": [
    {
      "set_id": "set-01",
      "title": "Bootstrap Project",
      "mode": "parallel",
      "tasks": [
        {
          "id": "t1",
          "name": "Generate Title",
          "model_profile": "fast",
          "status_line": "titling…",
          "success_criteria": "5–10 words, action + outcome",
          "steps": [
            { "type": "chat", "prompt": "Propose a concise project title." }
          ]
        },
        {
          "id": "t2",
          "name": "Run Lints",
          "model_profile": "fast",
          "status_line": "linting…",
          "steps": [
            { "type": "exec", "cmd": "cargo", "args": ["clippy","--all-targets","--all-features","--","-D","warnings"] }
          ]
        }
      ]
    }
  ]
}
```

3) **Run an MCP tool and a TaskSet** (CLI surface lives in `codex`; examples):

```bash
# Example: start an MCP server (stdio) and bridge it
codex mcp serve --stdio

# Example: connect to a child MCP server configured in .codex/config.toml
codex mcp connect --server everything

# Example: run a TaskSet by spec path (main model updates after set completes)
codex tasks run --file .codex/tasks/2025-09-01/SESSION-UUID/set-01.json
```

> **Note:** Prefer `codex` subcommands for all interactions. Transitional `codex-*` helper bins are allowed *only if explicitly requested* for bring‑up and should be folded back into `codex`.

---

## Architecture

### Identity & Integration

- `annex` is **not** a standalone binary. It is a **feature‑gated extension** compiled **into** the `codex` tool via the `codex-rs` submodule.
- Core components provided by this repo:
  - **ConfigManager** · **HookRegistry** · **SlashRegistry** · **TaskSetRunner** · **Compactor** · **SessionLogWriter**
- **Scope guard:** Touch the Codex submodule only when necessary. When required, mark changes as **[SUBMODULE CHANGE]**, provide an upstream‑ready patch, and ship a temporary shim in `annex`.

### Protocol Surfaces (Agents IO)

| Surface | Role(s) | Transport(s) | Status | Purpose |
|---|---|---|---|---|
| **MCP** (Model Context Protocol) | server **and** client | stdio · SSE · Streamable HTTP | **Implemented** | Primary tool bridge to **HookRegistry / TaskSetRunner / SlashRegistry**. Child‑process stdio client supported. |
| **ACP** (Zed Agent Client Protocol) | server | stdio | **Scaffolded** | Bridges Zed ACP agent semantics to TaskSetRunner/TODO/hooks. Handlers finalized as ACP crate stabilizes. |
| **A2A** (Agent‑to‑Agent) | server | HTTP · SSE | **Planned** | Implement via `jsonrpsee` with conformance **TCK**; methods mapped to task/session primitives. |
| **AGP** (OpenAI Agent Protocol) | server | HTTP | **Planned** | Code‑gen from OpenAPI; implement `/threads`, `/runs`, `/store` on session/TODO stores. |

### High‑Level Flow

```
User / Client
   │
   ▼
codex CLI  ──(annex features enabled)───────────────────────────────────────────────┐
   │                                                                               │
   ▼                                                                               │
TaskSetRunner  ⇄  HookRegistry  ⇄  SlashRegistry  ⇄  ConfigManager                │
   │                ▲                 ▲                ▲                           │
   │                │                 │                │                           │
   ├── Steps: chat/exec/mcp_call/git ┘                │                           │
   │                                                 │                           │
   ├─ chat → model (via profiles/overrides)          │                           │
   ├─ exec → shell/cargo/bash                        │                           │
   ├─ mcp_call → MCP bridge (server/client)          │                           │
   └─ git → VCS actions                               │                           │
   │                                                                               │
   ▼                                                                               │
Compactor (auto-compact focus instruction only)                                     │
   │                                                                               │
   ▼                                                                               │
SessionLogWriter (.codex/sessions/<date>/<SESSION-UUID>/{session.json,jsonl}) ─────┘
```

---

## Configuration Model (TOML config + JSON data)

**Precedence:** `workspace‑toml > user‑toml > system‑toml > runtime‑ephemeral`.

- `.codex/config.toml` governs:
  - `[models.default]` (primary), `[models.overrides.*]` (role‑based: `title`, `session_name`, `compact`, `meta_prompt`, `task_status`), and `[models.profiles.*]` (e.g., `fast`, `heavy`, `google`, `anthropic`).
  - Each entry may set `name`, `base_url`, `api_key_env` or `api_token_env`, optional `extra_headers`.

**MCP servers** should be declared in TOML under `[mcp.servers.*]`. Slash commands can patch workspace scope.

**Layout of `.codex/`**:

```
.codex/
  config.toml        # main config (models, shell, sessions, hooks, slash, mcp)
  hooks/             # *.toml hook definitions
  slash/             # *.toml alias/macro/builtins
  tasks/             # dated TaskSet specs (JSON files)
  todos/             # JSON TODO store (path configurable)
  sessions/          # session logs (JSON + JSONL)
```

---

## Task Sets (Agent Plans)

- **All tasks belong to a TaskSet.**  
- A set runs in **parallel** or **sequential** mode.
- **UI contract** (rendered by the host): square status, lane summary + task # + model label, **one‑line live status**.
- **Main model gets updated only after the entire TaskSet completes.** (Prevents leaking mid‑state outputs.)

**Spec path:** `.codex/tasks/<YYYY‑MM‑DD>/<SESSION‑UUID>/set‑XX.json`

**Schema (informal):**

```yaml
sets:
  - set_id: string
    title: string
    mode: parallel | sequential
    tasks:
      - id: string
        name: string
        model_profile: string # or 'profile'
        status_line: string   # template for live progress
        success_criteria: string
        on_error: continue | stop | retry:N
        steps:
          - type: chat
            prompt: string
            model_profile: string  # optional override
          - type: exec
            cmd: string
            args: [string,...]
          - type: mcp_call
            server: string
            method: string
            payload: {}
          - type: git
            action: string
            args: [string,...]
```

**Bridge functions (conceptual):**

- `do_chat(model_name, base_url, prompt) -> text`
- `do_exec(cmd, argv[]) -> {status, preview}`
- `do_mcp(server, method, payload) -> json`
- `do_git(action, args[]) -> {status}`

---

## Hooks (Lifecycle Policy)

**Events:** `PreExec`, `PostExec`, `PreMcp`, `PostMcp`, `TaskStart`, `TaskEnd`, **Git events**.

- If a rule with `deny_on_fail: true` fails or denies, **stop** and report the reason.
- On `TaskEnd`, run a prompt (profile `heavy`) to produce a **one‑line status** (“achievements + blockers”).
- Append exec lines to `.codex/audit.log` (see example in Quick Start).

**Location:** `.codex/hooks/*.toml`.

---

## Slash Commands

**Policy:** Prefer **alias** and **macro**; short, verb‑first names.  
**Location:** `.codex/slash/*.toml`.

- **MCP management:** `/mcp-add`, `/mcp-enable`, `/mcp-disable` patch **workspace** scope.
- Example macro `quick-title` provided above.

---

## Auto‑Compact

- **When:** `TaskEnd` (and optionally mid‑task if auto‑enabled).
- **Output:** **instruction‑only** (focus string). The compaction pipeline aggregates sources and produces final summaries outside this hook.
- **Context:** reference TODO state + session activity (completed vs pending, diffs, blockers).

---

## Sessions & Privacy

- **Path:** `.codex/sessions/<date>/<SESSION‑UUID>/session.json` and `session.jsonl`
- **Format:** JSON (array) and JSON Lines (append). Both written by default.
- **Redact:** patterns `*KEY*`, `*TOKEN*`, `*SECRET*`, `*PASSWORD*`.
- **Auto‑purge:** respect `auto_purge_days` if configured.
- **Observability:** use `SessionLogWriter` for chat, exec, file refs, meta. (OTEL/Langfuse optional in future.)

---

## Naming Conventions

- **Titles:** 5–10 words, **action + outcome**, **no emojis/brackets**.
- **Session names:** kebab‑case, ≤ 40 chars, include main intent.
- Always use `models.overrides.title` and `models.overrides.session_name` to generate these values.

---

## Ask vs Proceed

- **Proceed** if schema/path is unambiguous and safe defaults exist (e.g., `profile: fast` for interactive tasks).
- **Ask once** if potentially breaking (workspace members, resolver, API contracts) or risks overwriting maintained regions.
- **Defaults:** If details are missing, choose safe defaults, clearly **state them**, and continue.

---

## Submodules & External Protocol Deps

- Primary submodule: `external/openai-codex` (sparse: `codex-rs`, `codex-cli`), **pin commit** for deterministic builds.
- Protocol deps (sparse under `external/`):
  - `mcp-rust-sdk` (`crates/`, `README.md`)
  - `agent-client-protocol` (ACP) (`rust/`, `schema/`, `Cargo.toml`, `README.md`)
  - `agent-protocol` (AGP) (`openapi.json`, `api.html`, `server/`, `README.md`)
  - `a2a` (`specification/`, `README.md`)
- **Advancing upstream:** `git submodule update --remote --recursive` then commit pointer. If sparse paths change, document exact sparse‑checkout commands.

---

## Build & Install

**Make targets (expected):**

- `init` — bootstrap externals; set sparse; init Codex submodule
- `patch` — apply Codex feature/file patches for annex
- `build` — `cargo build --workspace`
- `install` — install Codex with `annex*` features; **no annex bin**
- `update` — sync & bump submodules; commit pointer

**Smoke checks (examples):**

```bash
codex --version
codex mcp --help
codex tasks --help
```

---

## CLI Surfaces (Examples)

> **Guidance:** Prefer **codex subcommands**. Individual names/flags may evolve; keep this section aligned with repo help output.

- **MCP**
  - `codex mcp serve --stdio` — start an MCP server over stdio.
  - `codex mcp serve --sse --port 8765` — start an MCP server over SSE/HTTP.
  - `codex mcp connect --server <name>` — connect to configured server (from `.codex/config.toml`).
- **ACP (Zed)**
  - `codex acp serve --stdio`
- **A2A (planned)**
  - `codex a2a serve --http --port 8787 --sse`
- **AGP (planned)**
  - `codex agentproto serve --http --port 8790`

---

## Security & Data Handling

- **No secrets in code or TOML.** Reference environment variables (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, etc.).
- Redaction patterns are enforced in session logs.
- Prefer least‑privilege env vars; never hardcode API keys or tokens.
- Audit trails (e.g., `.codex/audit.log`) are append‑only; review regularly.

---

## Extending Agents

### 1) Add an external MCP server

1. Declare server in `.codex/config.toml` (stdio or TCP/SSE) under `[mcp.servers.*]`.
2. Enable via `/mcp-enable <name>` (workspace scope).
3. Call with a Task step:

```yaml
- id: t3
  name: Use MCP Tool
  model_profile: fast
  status_line: "calling mcp…"
  steps:
    - type: mcp_call
      server: everything
      method: tools/search
      payload:
        query: "hello world"
```

### 2) Slash helpers

- Short, verb‑first names (`/todo`, `/ship`, `/lint`).
- Use macros to sequence complex actions across config and tasks.

### 3) Hooks

- Use `PreExec`/`PostExec` for deterministic shell build steps.
- `TaskEnd` prompt produces a **single‑line** status for the UI lane.

---

## Decision Matrix: MCP vs ACP vs A2A vs AGP

| Use Case | Choose |
|---|---|
| Tooling inside your local workspace, child processes, CLI utilities | **MCP** (stdio) |
| Editor‑driven agent integrations (Zed ACP) | **ACP** |
| Cross‑agent orchestration over HTTP/SSE, multi‑tenant | **A2A** (when implemented) |
| Compatibility with OpenAI Agent Protocol clients | **AGP** (when implemented) |

---

## Operational Recipes

### Run a parallel TaskSet and aggregate status

```yaml
sets:
  - set_id: set-02
    title: Build & Test
    mode: parallel
    tasks:
      - id: build
        name: Build
        model_profile: fast
        status_line: "building…"
        steps:
          - type: exec
            cmd: cargo
            args: ["build", "--workspace"]
      - id: test
        name: Test
        model_profile: fast
        status_line: "testing…"
        steps:
          - type: exec
            cmd: cargo
            args: ["test", "--workspace", "--", "--nocapture"]
```

```bash
codex tasks run --file .codex/tasks/2025-09-01/SESSION-UUID/set-02.json
```

### Append audit and summarize outcomes (hooks)

See **Quick Start** hook TOML. The `summarize-task` rule uses a **heavy** model profile to render a one‑liner per task completion.

---

## Error Handling & Policies

- **`deny_on_fail: true`** on a hook **halts** the pipeline and surfaces the reason.
- **`on_error`** at task‑level can be `continue`, `stop`, or `retry:N`.
- **Main model update** is deferred until **all tasks** in the set resolve, even if individual tasks fail (subject to policy).

---

## Testing & Conformance

- **Smoke:** `codex mcp --help`, `codex tasks --help`, `codex --version`.
- **A2A TCK (planned):** conformance suite once A2A is available.
- **AGP (planned):** endpoint contract tests generated from OpenAPI.

---

## Migration & Compatibility

- **TOML is source‑of‑truth for config.** JSON/JSONL for data.
- Avoid hardcoding model IDs in code. Always route via `[models]` profiles and overrides in `.codex/config.toml`.
- When advancing the Codex submodule:
  - Pin the commit; document sparse checkout; include a rollback note.
  - Provide a **[SUBMODULE CHANGE]** patch + temporary shim if we need to bridge time to upstream merge.

---

## Troubleshooting

- **MCP server not found:** ensure `.codex/config.toml` has `[mcp.servers.<name>] enabled = true`; re‑run `/mcp-enable <name>`.
- **No model credentials:** check env vars like `OPENAI_API_KEY`. Never store in TOML.
- **Hooks not firing:** confirm event names (`post_exec`, `task_end`) and that the hook file is in `.codex/hooks/`.
- **Task never updates main model:** expected until TaskSet completes; check set mode (`parallel|sequential`), long‑running steps, and `on_error` policy.

---

## Glossary

- **TaskSet** — a collection of tasks executed in `parallel` or `sequential` mode with a single committed main‑model update at the end.
- **Hook** — a policy‑driven reaction to lifecycle events; can run prompts or shell commands and may deny on failure.
- **Slash** — alias/macro command system, stored in TOML, driving common actions and config changes.
- **Compactor** — produces **focus instruction only** at TaskEnd; the pipeline composes summaries elsewhere.
- **SessionLogWriter** — append‑only event recorder for sessions.
- **MCP/ACP/A2A/AGP** — protocol surfaces for tool & agent interop (see matrix).

---

## Design Invariants (Non‑Negotiable)

- **TOML‑first** configuration with clear precedence and no hidden overrides.
- **Feature‑gated integration** into `codex`; **no separate annex binary**.
- **Main model update only after TaskSet completion**.
- **No secrets** in code/config; env var usage is mandatory.
- **Minimal diffs** in patches with clear rationale (“Why this? Why now?”).

---

## Closing Notes

Artifacts under this doc aim to be *directly usable*. If you encounter drift between this document and behavior, treat it as a **bug** in one or the other and raise it with a delta description (what changed, why, and impact). Keep session logs, hook policies, and model routing tidy—your future self will thank you.
