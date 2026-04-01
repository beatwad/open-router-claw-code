# OpenRouter Claw Code

<p align="center">
  <img src="assets/clawd-hero.jpeg" alt="Claw" width="300" />
</p>

<p align="center">
  <strong>Better Harness Tools, now working with any model from OpenRouter instead of Anthropic models only.</strong>
</p>

## Overview
This is a fork of the project [https://github.com/instructkr/claw-code/](https://github.com/instructkr/claw-code/).

Project OpenRouter Claw Code aims to create a highly capable CLI agent harness. **We have successfully updated this project to work with any model from OpenRouter, rather than being restricted to Anthropic models.** 

Currently, the repository contains two parallel tracks:
1. **The Functional Agent (Rust):** A fully working, high-performance CLI application that executes tools, maintains context, and talks to OpenRouter.
2. **The Porting Workspace (Python):** A clean-room Python rewrite currently in progress that captures the architectural patterns of the agent harness.

---

## 🚀 Getting Started: Using the Functional Agent (Rust)

To actually run the AI agent, interact with your codebase, and use OpenRouter models, you will use the Rust implementation. You will need [Rust installed](https://rustup.rs/) on your system.

### 1. Set your OpenRouter API Key
The agent authenticates using your OpenRouter API key. Export it in your terminal:

```bash
export OPENROUTER_API_KEY="sk-or-v1-..."

# Optional: If you are using a custom proxy endpoint
# export OPENROUTER_BASE_URL="https://your-custom-proxy.com/api"
```

### 2. Start the Interactive REPL
Navigate to the `rust` directory and run the application:

```bash
cd rust/
cargo run --release
```
*This will drop you into the interactive `claw` REPL where you can type prompts or use slash commands (like `/help` or `/status`).*

### 3. Using Specific OpenRouter Models
By default, the agent tries to use an Opus alias, but you can pass any OpenRouter model string or use built-in aliases (like `gpt4o`, `deepseek`, `sonnet`, `haiku`):

```bash
# Using a built-in alias:
cargo run --release -- --model google/gemini-3-flash-preview

# Using an exact OpenRouter model ID:
cargo run --release -- --model meta-llama/llama-3-70b-instruct
```

### 4. One-Shot Prompt Mode
To run a single command and exit without entering the REPL:
```bash
cargo run --release -- prompt "Analyze the files in this directory and summarize them"
```

---

## 🐍 Development: Python Porting Workspace

The main source tree (`src/`) contains the active Python porting workspace. The current Python workspace is not yet a complete one-to-one replacement for the Rust system, but it is the primary implementation surface for the rewrite.

### Repository Layout

```text
.
├── rust/                               # Functional, high-performance Rust CLI agent
├── src/                                # Python porting workspace (WIP)
│   ├── __init__.py
│   ├── commands.py
│   ├── main.py
│   ├── models.py
│   ├── port_manifest.py
│   ├── query_engine.py
│   ├── task.py
│   └── tools.py
├── tests/                              # Python verification
└── README.md
```

### Python Workspace Overview

The new Python `src/` tree currently provides:

- **`port_manifest.py`** — summarizes the current Python workspace structure
- **`models.py`** — dataclasses for subsystems, modules, and backlog state
- **`commands.py`** — Python-side command port metadata
- **`tools.py`** — Python-side tool port metadata
- **`query_engine.py`** — renders a Python porting summary from the active workspace
- **`main.py`** — a CLI entrypoint for manifest and summary output

### Python Quickstart

If you want to explore the Python rewrite codebase, you can run the following commands from the **root of the repository** (requires Python 3):

```bash
# Render the Python porting summary:
python3 -m src.main summary

# Print the current Python workspace manifest:
python3 -m src.main manifest

# List the current Python modules:
python3 -m src.main subsystems --limit 16

# Inspect mirrored command/tool inventories:
python3 -m src.main commands --limit 10
python3 -m src.main tools --limit 10

# Run verification tests:
python3 -m unittest discover -s tests -v
```
```