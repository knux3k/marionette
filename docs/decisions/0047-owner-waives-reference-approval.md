# 0047: An owner may waive reference image approval for named providers

Date: 2026-11-12. Status: accepted.

## Context

Decision 0045 makes a person approve each reference image before a paid 3D provider spends
credit on a model made from it (`<name>.approval.json`, written only by
`marionette generate approve`). The project owner found the step too heavy for their own work,
asked for model generation to run without it, and left rerolling weak images to the agent.

## Decision

- The personal config file (never a project, never an environment variable) may hold
  `"waive_reference_approval": ["tripo", ...]`, provider names, any case. A model made by a
  listed provider skips the approval check. Images never needed approval, so listing an image
  provider such as `qwen` only records intent.
- Provenance keeps `approved: false` (no person approved) and adds `approval_waived: true`, so
  every asset still says how it was cleared. The field is left out when false, so earlier
  provenance files are unchanged.
- The default stays: without the setting, the approval gate works as in 0045. Spend caps and the
  key rules are unaffected.
- Code: `approval::waived_providers`, used by the plan and the run (`paid_checks`).

## Consequences

- Whoever generates for that owner judges the reference images and rerolls weak ones before the
  model call, since no person looks first.
- Turning it off: delete the line from the personal config.
