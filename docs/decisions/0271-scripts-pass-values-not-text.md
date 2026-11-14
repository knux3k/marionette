# 0271: Scripts pass component values directly, not as JSON text

Date: 2026-11-14. Status: accepted.

## Context

A script's `self.get` and `self.set`, its saved state, its timers and its inbox all crossed
between Rust and the script engine as JSON text, written on one side and parsed on the other,
every tick, whether or not anything had changed. Measured in a release build on a 4-core Linux
container (`perf_sim`, 300 ticks): 1,000 scripts with an empty `update` 6.7 ms per tick; 1,000
scripts that read, change and write their `Transform` 34.6 ms. A profile put about three
quarters of that in text conversion. No budget test covered scripts.

## Decision

- `marionette_script::convert` turns a component into a script value and back without text.
  - Reading: a component that is a plain struct of numbers, booleans and strings (up to 64
    leaves) gets a plan on first use. The host writes its numbers into one shared
    `Float64Array` and a generated constructor builds the object. Other components are
    converted by a serde serializer that builds script values; what it cannot express falls
    back to text.
  - Writing: the script object is written over a copy of the component field by field
    (`patch`). If the object is not exactly a plain object with every field present, a general
    converter builds the JSON value; if that declines too (class instances, `toJSON`, proxies,
    integers beyond 2^53, depth over 32), the old text path runs.
  - Every number goes through the same rounding as the text path, so results and the state
    hash do not change. Tests compare both paths on every registered component and on 60,000
    random numbers.
- A turn does only what it needs: contacts, timers and the inbox are passed only when present,
  the timer table is made on first use, and a turn that changed neither state nor timers
  returns nothing to save.
- Budgets in `perf_budgets` (`script_budgets`): 1,000 empty scripts 5 ms, 1,000 moving scripts
  18 ms per tick.

## Result

Same container: empty 6.7 ms to 2.6 ms; moving 34.6 ms to 10.8 ms. The drift hash and the
scripted example's hash are unchanged.

## Why

- The text path stays as the fallback and the reference, so behaviour is defined in one place
  and the fast paths are checked against it instead of being trusted.
- A generated write plan in script code was measured slower than the native `patch`, so only
  reading uses a plan.

## Not done yet

- A script still costs about 2.6 microseconds per entity per tick before it does anything, and
  about 11 for a read and a write. Thousands of entities per tick belong in a job (decision
  0030, skill `marionette-write-job`) or a Rust module (decision 0062); scripts are for
  behaviour on tens to hundreds of entities. Letting a job read and write component columns
  directly would close the gap for simple per-entity movement and is not built.
- Allocation counts per tick were not measured again.
