# 0142: Every commit scanned for secrets in CI (gitleaks), with the providers' key formats

Status: accepted (2026-11-17)

## Context

Before the first public release the owner required proof that no API key is anywhere in the
repositories, and a gate that keeps it that way. The engine reads provider keys (Anthropic,
DashScope/Qwen, Tripo) from the environment or a keys file outside every project, and refuses a
keys file inside one (decision 0045), but nothing checked commits.

## Decision

- A `secret scan` job in `ci.yml` on every pull request and push to main: gitleaks 8.28.0 (a
  pinned release, checksum checked) over the whole history with `.gitleaks.toml`. The job is in
  the `CI gate`'s needs, so a finding blocks merging. The whole history takes about two seconds.
- `.gitleaks.toml` extends the default rules with the key formats the engine uses: `sk-ant-`
  (Anthropic), `sk-` (DashScope and OpenAI-compatible endpoints), `tsk_` (Tripo), and a provider
  key variable assigned a value.
- Allowlists are narrow: the `cache_key` field of generated-asset provenance records (a SHA-256
  of the request) and the key-file parser's test file, whose keys are visibly fake.
- The audit before this decision: gitleaks, trufflehog and a key-format scan of every object in
  every branch, tag and pull request ref of the engine, assets and game repositories found only
  those false positives; pull request and issue text, the release binary and the project's
  shared files and bundles had none.

## Alternatives

- **trufflehog in CI**: also run in the audit; slower to configure narrowly, and its git source
  needed a non-bare clone. gitleaks with a checked-in config is simpler to run locally.
- **gitleaks-action**: needs a license key for organizations and adds a moving dependency; the
  pinned binary does the same.
- **Scanning only the pull request's commits**: faster in theory, but the whole history already
  takes two seconds and also catches anything pushed around the check.

## Consequences

- A test that needs a fake key must use an obviously fake value or live in the allowlisted
  test file; a new allowlist entry is a reviewed change to `.gitleaks.toml`.
- A real key that reaches a pull request must still be revoked at the provider: the scan blocks
  the merge, not the push.
