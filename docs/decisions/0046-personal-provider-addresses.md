# 0046: Personal provider addresses and models

Date: 2026-11-12. Status: accepted.

## Context

A Model Studio subscription plan key (prefix `sk-sp-`) is refused by every regular Qwen
address (international, China, US) with HTTP 401. Such a plan has its own address
(`https://token-plan.<region>.maas.aliyuncs.com/...`), its own image API and its own model list
(for example `qwen-image-3.0-pro`). A Model Studio "coding plan" address
(`coding-intl.dashscope.aliyuncs.com/apps/anthropic` or `/v1`) serves chat models only and
cannot make images.

## Decision

- **Where it lives.** Address and model are facts about one person's account, so they go in the
  per-user config file, never in a project: `"provider_endpoints": {"qwen": {"url": ..., "model":
  ...}}`, or a plain string for the address alone. `url` is an `https://` address or the path of
  a text file whose first non-empty line is one. `MARIONETTE_<NAME>_ENDPOINT` and
  `MARIONETTE_<NAME>_MODEL` override the file. Code: `marionette_providers::endpoints`.
- **Model.** The personal model fills only a `providers.json` choice that names the provider and
  pins no model. It is applied when the config is loaded (`generate_plan`, `generate_run`,
  `provider_generate`), so cache keys and provenance name the model that really ran.
- **Qwen on a personal address.** One synchronous call to
  `{host}/api/v1/services/aigc/multimodal-generation/generation` (prompt in
  `input.messages[0].content[0].text`, same `parameters` as before), image address from
  `output.choices[0].message.content[*].image`, then the download. No region search, no polling.
  `Ctx` gained `endpoint`.
- **Plan terms.** Subscription plans may limit use to interactive tools. Whether a plan key is
  used for generation is the account owner's decision; the engine only follows the config.

## Consequences

- The estimate stays the per-image price of decision 0045; plan usage is counted in the plan's
  own credits, which the engine cannot read.
- UNVERIFIED until the first live call: the synchronous API's field names come from the
  provider's documentation, not a live capture.
