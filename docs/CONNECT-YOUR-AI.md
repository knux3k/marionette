# Connect your AI to Marionette

Marionette is built to be driven by an AI. This guide is for people: how to hook up the AI
you already use, and how to give the engine its own API keys for generating art. No part of the
engine needs an AI to run; everything below is optional.

There are three different kinds of AI involved, and they are set up separately:

| | What it does | Who pays | Set up in |
| --- | --- | --- | --- |
| **Your coding agent** (Claude Code, Codex, Gemini CLI, Cursor, ...) | Builds and changes your game: edits scenes, writes scripts, runs tests, looks at screenshots | Your existing subscription to that app | [Section 1](#1-use-the-ai-app-you-already-pay-for) |
| **Engine providers** (Qwen, Tripo, Claude API) | Generate images, textures and 3D models when no existing asset fits | Your own API keys, under a spend cap you set | [Section 3](#3-api-keys-for-generating-art) |
| **AI while playing** (characters that talk, and so on) | Not built yet. Planned as an optional module that a game must turn on; off by default | | |

## 1. Use the AI app you already pay for

Your coding agent does the work; Marionette gives it the tools. The agent talks to the engine
through **MCP** (Model Context Protocol, the standard way coding agents get extra tools): the
command `marionette mcp` starts a small server that offers the engine's operations (create an
entity, run the game for 600 ticks, take a screenshot, run the tests, ...) as tools. Nothing is
sent anywhere by Marionette itself; the agent runs on your subscription exactly as it does for
any other project, and Marionette runs on your machine.

Agents that cannot use MCP can run the same operations on the command line,
`marionette op <name> '<json>'`, with the same results.

Every project made by `marionette make` or `project_init` also contains **skills**: short,
tested procedures for common tasks (write a script, light a scene, port Unity code, find a
slowdown). Agents that support skills (Claude Code reads `.claude/skills`, others read
`.agents/skills`) load the right one by themselves. Any agent can also list and read them through
the `skills_list` and `skills_get` tools.

### Claude Code

Claude Code works with a Claude Pro or Max plan, or with Anthropic API credits; whichever you
use for Claude Code is what pays. Marionette needs no key of its own for this.

1. Install Claude Code (see claude.com/claude-code) and sign in.
2. In your game folder, register the engine once:

   ```bash
   cd fox
   claude mcp add marionette -- marionette --project . mcp --tools core --lean
   ```

   Or commit a `.mcp.json` file in the game folder, so everyone who opens the project gets it:

   ```json
   {
     "mcpServers": {
       "marionette": {
         "command": "marionette",
         "args": ["--project", ".", "mcp", "--tools", "core", "--lean"]
       }
     }
   }
   ```

3. Run `claude` in the game folder and ask for what you want in plain words. `/mcp` inside
   Claude Code shows whether the server is connected.

### Claude Desktop

Claude Desktop starts MCP servers outside your project folder, so give the full path to the
game. In Settings, Developer, Edit Config, add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "marionette": {
      "command": "marionette",
      "args": ["--project", "C:\\Games\\fox", "mcp", "--tools", "core", "--lean"]
    }
  }
}
```

If `marionette` is not on the `PATH` that Claude Desktop sees, put the full path to the binary in
`command`. Restart Claude Desktop afterwards.

### Other agents

The server is the same for every client: the command `marionette`, the arguments
`--project <game folder> mcp`, spoken over standard input and output. Where each app keeps its
MCP settings (check your app's own docs, as these formats change):

| App | Where | Shape |
| --- | --- | --- |
| OpenAI Codex CLI | `~/.codex/config.toml`, or `codex mcp add marionette -- marionette --project . mcp` | `[mcp_servers.marionette]` with `command` and `args` |
| Gemini CLI | `.gemini/settings.json` in the project | `"mcpServers": {"marionette": {"command": ..., "args": [...]}}` |
| Cursor | `.cursor/mcp.json` in the project | `"mcpServers"` as above |
| VS Code (Copilot agent mode) | `.vscode/mcp.json` in the project | `"servers": {"marionette": {"type": "stdio", "command": ..., "args": [...]}}` |
| Anything else | Start the process yourself | Newline-delimited JSON-RPC 2.0 on stdin and stdout |

For agents that read an instructions file, point them at the project's `docs/ai/START-HERE.md`.
A project made with `project_init` also has an `AGENTS.md` for this.

### Choosing which tools the agent sees

Every tool an agent can see costs part of its context on every request. The flags keep that small:

- `--tools core` lists only the edit, run, observe and test tools (projects, scenes, entities,
  components, resources, sessions, observe, test, batch, skills, handoff). Add groups as the game
  needs them: `--tools core,anim,audio,pool`. A group is an operation's name up to its first
  underscore, so `anim_play` is in `anim`. Leave the flag out for everything.
- `--lean` leaves the output schemas out of the listing. Results are the same.
- `--read-only` gives an agent that can look and test but not change anything:
  `marionette --project . --read-only mcp`.

### Several agents at once

Each project carries handoff files in `docs/ai/`. An agent starts a session with
`handoff_status`, claims the files it will change with `handoff_claim` (refused when another
agent holds them) and finishes with `handoff_note`. The installed skill
`marionette-handoff` explains the routine to the agent.

## 2. What the agent can and cannot do on your machine

- Scene edits go through validated operations: a wrong component name or value is refused with
  an error that says how to fix it, so a scene file never ends up half-broken.
- The engine never spends money on its own. Paid generation needs a key and a spend cap that only
  a person can set, and turning an image into a paid 3D model needs a person to type approval at
  a terminal (section 3).
- Windows and sound: `marionette run` and `marionette edit` open a window. Agents normally use the
  headless commands and screenshots instead.
- Your coding agent's own permission settings still apply on top of this; Marionette does not
  change them.

## 3. API keys for generating art

When a game needs a model or texture that is not in your project, your own asset library or the
CC0 pool, the engine can generate it from a recipe. These calls go from Marionette straight to the
provider, with your key, and are billed by the provider to you.

| Provider | Makes | Key variable | Get a key |
| --- | --- | --- | --- |
| `qwen` | Images and textures (Qwen Image) | `DASHSCOPE_API_KEY` | Alibaba Cloud Model Studio |
| `tripo` | 3D models from text or an image | `TRIPO_API_KEY` | Tripo |
| `claude` | Text (language), for engine features that ask for it | `ANTHROPIC_API_KEY` | Anthropic Console |
| `mock` | Simple test images and models, offline and free | none | built in, the default |

The `claude` provider uses the Anthropic API with an API key, which is billed separately from a
Claude Pro or Max plan. Its request format follows the API documentation but has not yet been
checked against the live service; Qwen and Tripo have been.

### Step by step

1. **Put the key outside every project**, so it can never be committed. Either set an
   environment variable (`DASHSCOPE_API_KEY=...`), or keep keys in a file and name that file in
   your user config:

   - Windows: `%APPDATA%\marionette\config.json`
   - Linux: `~/.config/marionette/config.json`

   ```json
   {
     "keys_file": "D:\\secrets\\marionette-keys.txt",
     "spend_cap_usd": 5,
     "provider_caps_usd": { "tripo": 10 }
   }
   ```

   The keys file holds `NAME=value` lines (or a JSON object). You can also name one file per key
   with `"key_files": {"DASHSCOPE_API_KEY": "<path>", "TRIPO_API_KEY": "<path>"}`. A keys file
   inside a project folder is refused and never read, and no command ever prints a key.

2. **Set a spend cap.** The cap starts at zero, which refuses every paid call. Only you can raise
   it, with `spend_cap_usd` (total) or `provider_caps_usd` (per provider, a number or
   `"unlimited"`) in the user config, or `MARIONETTE_SPEND_CAP_USD` and
   `MARIONETTE_SPEND_CAP_<PROVIDER>_USD`. No operation can change it, so an agent cannot raise its
   own budget. What has been spent is kept per project in `.marionette/spend.jsonl`.

3. **Choose providers for the project** (written to `providers.json` in the game folder, which
   never holds keys):

   ```bash
   marionette provider use image qwen
   marionette provider use model3d tripo
   marionette provider status        # ready or not, spending and caps; never shows a key
   ```

4. **Plan, approve, run.**

   ```bash
   marionette generate plan            # every call, cached or not, with its estimated cost
   marionette generate approve --all   # a person confirms reference images for paid 3D models
   marionette generate run
   ```

   The same request is never paid twice: outputs are cached by request, and each generated asset
   gets a provenance record (provider, prompt, cost). Generated assets are never added to the
   public CC0 pool, since who owns them depends on each provider's terms.

Prebuilt binaries can reach the network. A binary built from source needs
`cargo install --locked --path crates/marionette-cli --features net`; without that feature the
engine cannot make a network call at all, which some people prefer.

### Qwen plans and regions

A pay-as-you-go Model Studio key works from any region; the engine tries the international,
China and US addresses in turn. A Model Studio plan key (it starts with `sk-sp-`) needs the plan's
own address and image model in the user config:

```json
{ "provider_endpoints": { "qwen": { "url": "https://<your plan address>/compatible-mode/v1", "model": "qwen-image-3.0-pro" } } }
```

`MARIONETTE_QWEN_ENDPOINT` and `MARIONETTE_QWEN_MODEL` override the file.

## 4. Adding another provider

Providers are plug-ins in `crates/marionette-providers`. Each is one Rust file implementing the
`Provider` trait: `info()` (name, kinds, key variable, default models), `estimate_usd()` (the
upper cost of a request, checked against the cap before anything is sent) and `run()` (build the
HTTP request, read the answer). `qwen.rs` and `tripo.rs` are complete examples, including
asynchronous jobs that are polled. Register the new one in `all()` in `lib.rs`, add tests with
recorded responses (no network in tests), and it appears in `provider list`, `provider use` and
generation. The rules a provider must follow are in
[decision 0019](decisions/0019-providers.md) and [decision 0045](decisions/0045-asset-generation.md).
Pull requests for new providers are welcome.

## Troubleshooting

| Message | What to do |
| --- | --- |
| `provider.key_missing` | The key variable is not set and no keys file supplies it; check `marionette provider list` |
| `provider.spend_cap` | Raise the cap in your user config (the default is zero) |
| `provider.network_disabled` | A source build without `--features net`; rebuild with it |
| `provider.keys_file_in_project` | Move the keys file out of the project folder |
| `generate.not_approved` | Run `marionette generate approve <name>` (or `--all`) at a terminal |
| The agent does not see the tools | Check that `marionette --version` works in the same terminal, then `/mcp` (Claude Code) or your app's MCP status page |
| Windows PowerShell 5.1 mangles JSON arguments | Run `marionette op` from Git Bash or `cmd`, or escape inner quotes as `\"` |
