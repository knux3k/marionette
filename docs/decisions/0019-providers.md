# 0019: Swappable AI providers, offline by default, spending gated twice

Date: 2026-11-12. Status: accepted.

## Decision

- **Crate.** `crates/marionette-providers` holds the provider interface, the plug-ins, the
  config file and the spend cap. It depends on core (errors as data) and assets (PNG encoder)
  only. Operations are in `crates/marionette-agent/src/providers.rs`; the CLI shortcut is
  `marionette provider list|status|use <kind> <provider> [--model m]`.
- **Interface.** Three kinds: `language` (text and code), `image` (PNG), `model3d` (GLB). A
  plug-in implements `Provider`: `info()` (name, kinds, key variable, network need, default
  model per kind, `verified`), `estimate_usd(model, request)` (an upper estimate) and
  `run(ctx, request)`. Requests and outputs are plain Rust values; one call is synchronous.
  Job-based services (Qwen, Tripo) submit, poll and download inside `run`, with the poll
  limit and the wait between polls passed in, so tests never sleep.
- **HTTP as data.** Network plug-ins build `HttpRequest` values and parse `HttpResponse`
  values; a `Transport` moves the bytes. Tests drive every plug-in through a scripted
  transport (request shape, answer parsing, polling, failures, timeouts). Header values that
  hold a key are marked secret and redacted in debug output.
- **No network transport in this build.** The only transport shipped is `NoNetwork`, so no
  code path in the engine can reach a paid API yet. Real calls arrive with a small HTTP
  transport in workstream W4, after the project owner approves the first paid call. This is
  the first gate on spending.
- **Spend cap, the second gate.** Before any paid call, `estimate + ledger total <= cap` must
  hold. The cap is set by a person only: `MARIONETTE_SPEND_CAP_USD`, else `spend_cap_usd` in the
  user config file (the file that already names the owned library), else zero, which refuses
  every paid call. No operation writes either. The ledger is
  `<project>/.marionette/spend.jsonl`, one line per paid call with the actual cost; an
  unreadable ledger refuses paid calls. Claude is priced from reported token usage; Qwen and
  Tripo at a fixed assumed price per job (UNVERIFIED).
- **Mock is the default.** Without `providers.json`, or for a kind it leaves out, every kind
  uses `mock`: offline, free and deterministic (integer arithmetic only), with output that
  depends on the model name and the request. Images are checkerboard PNGs, models are GLB
  boxes that load with the engine's loader, text names the model and a digest of the prompt.
  Hashes of mock output are pinned in the tests, so CI on both platforms proves byte equality.
- **One config file, one command.** `providers.json` at the project root: format
  `marionette.providers/1`, then `language`, `image`, `model3d`, each `{provider, model?}`.
  Unknown fields are refused (`provider.bad_config`), so a key cannot be stored in it.
  `provider_use` validates the provider name and that it offers the kind before rewriting the
  file canonically.
- **Keys.** Environment variables only: `ANTHROPIC_API_KEY` (claude), `DASHSCOPE_API_KEY`
  (qwen), `TRIPO_API_KEY` (tripo). Readiness reports whether a key is set, never its value.
- **Check order** in `generate`: request validity, provider and kind, key, network, spend cap;
  then the call; then the ledger. Nothing is sent before every check passes (tested).
- **Plug-in formats.** Claude: Messages API, `anthropic-version: 2026-10-21`, default model
  `claude-opus-5-5`, no sampling fields, refusals become `provider.refused`. Qwen: Model
  Studio (DashScope) asynchronous text-to-image, international endpoint, default model
  `qwen-image`, PNG only. Tripo: v2 task API, `text_to_model` or `image_to_model` by URL,
  `model_version` sent unless the model is `default`, GLB only. All three are marked
  `verified: false`: written from documentation, never called live.

## Why

- The brief asks for cloud providers with each user's own keys, local models allowed, one
  command to switch, and a spend cap; and the engine must build and pass tests with no provider
  configured and no network. A deterministic mock as the default satisfies the second and keeps
  generated content usable in tests and examples.
- Spending credits needs the owner's approval. Leaving the transport out entirely makes that
  structural rather than a matter of configuration, and the person-only cap keeps an agent
  from raising its own limit later.
- Treating HTTP as data lets the plug-ins be built and tested now, so the W4 change is a small
  transport plus a live check of each format.

## Rejected

- An HTTP client dependency now (for example `ureq` behind a feature flag): untestable here
  without spending, and it would make paid calls one configuration step away.
- Feature flags per plug-in: the plug-ins have no dependencies of their own yet, so a flag
  would only hide code from the build and the tests. A `net` feature can carry the transport.
- Storing the cap in `providers.json`: any operation that writes that file could raise it.
- Keys in a config file or keychain: the brief says environment variables only.

## Not done yet

- The network transport and live verification of the three formats (W4, with the owner's
  approval and a cap set).
- A local-model provider (for example an HTTP server on the same machine). It fits the same
  interface and needs the transport first.
- Tripo upload of a local image (only image URLs now), Tripo rigging, other image formats
  than PNG, streaming text.
- Spending per period (the cap applies to the ledger's whole total).
