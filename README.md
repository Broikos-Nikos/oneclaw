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
curl -sSL https://raw.githubusercontent.com/Broikos-Nikos/oneclaw/main/install.sh | bash
```

Or build from source:

```bash
git clone https://github.com/Broikos-Nikos/oneclaw.git
cd oneclaw
cargo build --release
cp target/release/oneclaw ~/.cargo/bin/
```

## Quick Start

```bash
# 1. Set up the main routing agent + optional channels
oneclaw onboard

# 2. Create specialist sub-agents (optional but recommended)
oneclaw create-agent developer --role "Software engineer"
oneclaw create-agent creative  --role "Writer and content creator"
oneclaw create-agent analyst   --role "Data analyst and researcher"

# 3. Start chatting (CLI)
oneclaw agent

# 4. Or start the background daemon (Telegram / WhatsApp listener)
oneclaw daemon
```

## Multi-Agent Delegation

When the main agent decides to delegate, it:
1. Calls `transfer_to_agent` with the sub-agent name and task description.
2. OneClaw spins up the named sub-agent and runs the task.
3. The sub-agent's result is fed back to the main agent via `continue_with_result`.
4. The main agent synthesizes all results and responds to the user.

This loop continues (up to 4 levels deep) until the main agent returns a final response.

```
User → Main agent → transfer_to_agent("developer", "write the API")
                         ↓
                    Developer agent runs, outputs code
                         ↓
     Main agent ← sub-agent result (fed back automatically)
         ↓
     Main agent → transfer_to_agent("analyst", "review the code")
                         ↓
                    Analyst agent reviews, outputs report
                         ↓
     Main agent ← sub-agent result
         ↓
     Main agent → final response to user (synthesized)
```

## Commands

| Command | Description |
|---|---|
| `oneclaw agent` | Interactive chat session |
| `oneclaw agent -m "message"` | Single-shot message |
| `oneclaw agent --agent developer -m "fix the bug"` | Force a specific agent |
| `oneclaw onboard` | Initial setup (config + main agent + channels) |
| `oneclaw create-agent <name>` | Create a new sub-agent |
| `oneclaw list-agents` | List all configured agents |
| `oneclaw daemon` | Start background daemon (channels + cron + heartbeat) |
| `oneclaw status` | Show configuration and agents |
| `oneclaw doctor` | Run diagnostics |
| `oneclaw sample-config` | Print sample config TOML |

## Configuration

Config lives at `~/.config/oneclaw/config.toml`:

```toml
# Main agent (required)
[providers.main]
kind = "anthropic"
api_key = "sk-ant-YOUR_KEY"
model = "claude-sonnet-4-20250514"
temperature = 0.7

# Sub-agents can have their own provider/model
[providers.developer]
kind = "openai"
api_key = "sk-YOUR_OPENAI_KEY"
model = "gpt-4o"
temperature = 0.3

# Or use a local model for a fast cheap sub-agent
[providers.quick]
kind = "ollama"
model = "llama3.2"
temperature = 0.5

[workspace]
path = "~/.oneclaw/workspace"

[agents]
souls_dir = "~/.oneclaw/agents"
```

Sub-agents without a `[providers.<name>]` section fall back to the main agent's provider.

### Provider Kinds

| Kind | Notes |
|------|-------|
| `anthropic` | Direct Anthropic API (Claude models) |
| `openai` | Direct OpenAI API (GPT-4o, o3-mini, etc.) |
| `ollama` | Local Ollama server (no API key needed) |
| `compatible` | Any OpenAI-compatible endpoint — set `base_url` |

## Channels

OneClaw supports Telegram and WhatsApp alongside the CLI. The daemon receives
messages, runs the full agent + delegation loop, and **replies back** to the user
in the same channel thread.

```toml
# Telegram — simple long-polling bot, no public URL needed
[channels.telegram]
bot_token = "123456:ABCdef-YOUR_TOKEN"  # from @BotFather on Telegram
allowed_users = ["*"]                   # or ["your_username"]

# WhatsApp — Meta Business Cloud API (requires public HTTPS webhook)
[channels.whatsapp]
access_token  = "YOUR_META_ACCESS_TOKEN"
phone_number_id = "YOUR_PHONE_NUMBER_ID"
verify_token  = "your-secret-string"
allowed_numbers = ["*"]                 # or ["+12125551234"]
webhook_port  = 8443
```

**Telegram setup:** message @BotFather → `/newbot` → copy the token.

**WhatsApp setup:**
1. Create a Meta app at https://developers.facebook.com/apps/
2. Enable WhatsApp Business API.
3. Set webhook URL to `https://<your-domain>:<port>/webhook` (needs public HTTPS).
4. Use ngrok or Cloudflare Tunnel during development.
5. Run `oneclaw daemon` to start the listener.

## Agent Souls

Each agent has a soul folder at `~/.oneclaw/agents/<name>/`:

```
~/.oneclaw/agents/
└── main/
    ├── identity.json    # Name, role, personality, instructions
    ├── SOUL.md          # Behavioral guidelines (free-form markdown)
    ├── USER.md          # User profile and preferences
    ├── TOOLS.md         # Tool permissions and limitations
    └── AGENTS.md        # Operational safety rules
```

The `identity.json` format:

```json
{
  "name": "OneClaw Developer",
  "role": "Software engineer and architect",
  "personality": "Precise, methodical, detail-oriented",
  "instructions": [
    "Write production-quality code with tests",
    "Read existing files before modifying them",
    "Verify changes with the shell tool"
  ],
  "strengths": ["Coding", "Debugging", "Architecture"],
  "style": "Technical and precise"
}
```

## Architecture Differences from ZeroClaw

| | ZeroClaw | OneClaw |
|---|---|---|
| **Agents** | Single agent | Multi-agent with router + delegation |
| **Delegation** | None | Main → sub → result fed back → loop |
| **Channels** | 25+ platforms | Telegram + WhatsApp |
| **Channel replies** | Supported | Replies back via same channel |
| **Providers** | OpenRouter only | Anthropic, OpenAI, Ollama, compatible endpoint |
| **Hardware** | GPIO, STM32, USB | Removed |
| **WASM plugins** | Supported | Removed |
| **Dependencies** | ~60 crates | ~20 crates |

## License

MIT OR Apache-2.0

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

### Provider Kinds

| Kind | Base URL | Notes |
|------|----------|-------|
| `openrouter` | `openrouter.ai/api/v1` | Access 100+ models via one key |
| `openai` | `api.openai.com/v1` | Direct OpenAI (GPT-4, etc.) |
| `anthropic` | `api.anthropic.com/v1` | Direct Anthropic (Claude, etc.) |

## Channels

OneClaw supports messaging platform channels alongside the CLI:

```toml
# Telegram — simple, works behind NAT
[channels.telegram]
bot_token = "123456:ABCdef-YOUR_TOKEN"  # from @BotFather
allowed_users = ["*"]                   # or ["your_username"]

# WhatsApp — requires Meta Business account + public HTTPS endpoint
[channels.whatsapp]
access_token = "YOUR_META_ACCESS_TOKEN"
phone_number_id = "YOUR_PHONE_NUMBER_ID"
verify_token = "oneclaw-verify"
allowed_numbers = ["*"]
webhook_port = 8443
```

Run `oneclaw agent` — channels start alongside the CLI via `tokio::select!`.

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
| **Channels** | 25+ (Telegram, Discord, etc.) | Telegram + WhatsApp |
| **Hardware** | GPIO, STM32, USB | Removed |
| **WASM plugins** | Supported | Removed |
| **Dependencies** | ~60 crates | ~20 crates |
| **Providers** | OpenRouter only | OpenRouter, OpenAI, Anthropic |

## License

MIT OR Apache-2.0 (same as ZeroClaw)
