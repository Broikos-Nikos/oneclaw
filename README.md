# 🦀 OneClaw

**Multi-agent AI assistant with router architecture.**

Fork of [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw), streamlined and rebuilt around a multi-agent paradigm.

## How It Works

OneClaw uses a **main agent** that acts as a router. When you send a message, the main agent decides whether to handle it directly or delegate to a specialized **sub-agent**.

- **Main agent** — Created at setup. Routes tasks and orchestrates multi-step work.
- **Sub-agents** — User-created. Any name, any role. Each has its own soul/identity.
- **Shared workspace** — All agents read/write the same workspace directory.

```
User → Main Agent → handles directly OR delegates to sub-agent
                  ↕                          ↕
             Shared Workspace (~/.oneclaw/workspace/)
```

### Multi-Step Example

A task like "Design and build a landing page" flows:
1. Main receives task → delegates structure to **developer** agent
2. Developer writes HTML/CSS, returns summary
3. Main delegates copy writing to **creative** agent
4. Creative reads existing files, writes copy
5. Main delegates finalization back to **developer**
6. Main synthesizes final response to user

## Install

```bash
curl -sSL https://raw.githubusercontent.com/YOUR_USERNAME/oneclaw/main/install.sh | bash
```

Or build from source:

```bash
git clone https://github.com/YOUR_USERNAME/oneclaw.git
cd oneclaw
cargo build --release
cp target/release/oneclaw ~/.cargo/bin/
```

## Quick Start

```bash
# 1. Set up the main agent
oneclaw onboard

# 2. Create sub-agents
oneclaw create-agent developer --role "Software engineer"
oneclaw create-agent creative --role "Writer and content creator"
oneclaw create-agent quick --role "Fast responder for simple tasks"

# 3. Start chatting
oneclaw agent
```

## Commands

| Command | Description |
|---|---|
| `oneclaw agent` | Interactive chat session |
| `oneclaw agent -m "message"` | Single-shot message |
| `oneclaw agent --agent developer -m "fix the bug"` | Force a specific agent |
| `oneclaw onboard` | Initial setup (config + main agent) |
| `oneclaw create-agent <name>` | Create a new sub-agent |
| `oneclaw list-agents` | List all configured agents |
| `oneclaw status` | Show configuration and agents |
| `oneclaw sample-config` | Print sample config TOML |

### Interactive Commands

| Command | Description |
|---|---|
| `/agent <name>` | Force routing to a specific agent |
| `/agent auto` | Return to automatic routing |
| `/agents` | List available agents |
| `/clear` | Clear conversation history |
| `/quit` | Exit |

## Configuration

Config lives at `~/.config/oneclaw/config.toml`:

```toml
# The main agent is required
[providers.main]
kind = "openrouter"
api_key = "sk-or-v1-YOUR_KEY"
model = "anthropic/claude-sonnet-4-20250514"
temperature = 0.7

# Sub-agents can have their own provider/model
[providers.developer]
kind = "openrouter"
api_key = "sk-or-v1-YOUR_KEY"
model = "anthropic/claude-sonnet-4-20250514"
temperature = 0.3

# Or use a cheaper model for simple tasks
[providers.quick]
kind = "openrouter"
api_key = "sk-or-v1-YOUR_KEY"
model = "meta-llama/llama-3.3-70b-instruct"
temperature = 0.5

[workspace]
path = "~/.oneclaw/workspace"

[agents]
souls_dir = "~/.oneclaw/agents"
```

Sub-agents without a `[providers.<name>]` section fall back to the main agent's provider.

## Agent Souls

Each agent has a soul folder at `~/.oneclaw/agents/<name>/`:

```
~/.oneclaw/agents/
├── main/
│   ├── identity.json    # Name, role, personality, instructions
│   └── SOUL.md          # Behavioral guidelines (free-form markdown)
├── developer/
│   ├── identity.json
│   └── SOUL.md
└── creative/
    ├── identity.json
    └── SOUL.md
```

Edit these files to customize agent behavior. The `identity.json` format:

```json
{
  "name": "OneClaw Developer",
  "role": "Software engineer and architect",
  "personality": "Precise, methodical, detail-oriented",
  "instructions": [
    "Write production-quality code",
    "Test your changes using the shell tool"
  ],
  "strengths": ["Coding", "Debugging", "Architecture"],
  "style": "Technical and precise"
}
```

## Architecture Differences from ZeroClaw

| | ZeroClaw | OneClaw |
|---|---|---|
| **Agents** | Single agent | Multi-agent with router |
| **Agent config** | One identity | Per-agent soul folders |
| **Routing** | Manual | Main agent decides |
| **Sub-agents** | Not supported | User-created, any name |
| **Channels** | 25+ (Telegram, Discord, etc.) | CLI only |
| **Hardware** | GPIO, STM32, USB | Removed |
| **WASM plugins** | Supported | Removed |
| **Dependencies** | ~60 crates | ~20 crates |
| **Platform integrations** | WhatsApp, Matrix, Lark, etc. | None (lean) |

## License

MIT OR Apache-2.0 (same as ZeroClaw)
