# Design: `fuel = 0` Unlimited Mode

**Issue:** #164
**Date:** 2026-02-17

## Problem

Setting `fuel` to a very high value makes the fuel budget behaviourally inert, but fuel
info still gets injected into LLM-visible prompts and hook payloads, wasting context
window tokens on meaningless bookkeeping.

## Design

`fuel = 0` is the sentinel for "unlimited / no tracking". This formalises an existing
idiom (users already set high values to approximate unlimited) with zero type-system
churn — `usize` stays `usize`.

### config.rs

- Update doc comments on `Config::fuel`, `LocalConfig::fuel`, and `ResolvedConfig::fuel`
  to document that `0` means unlimited.
- No type or default value changes.

### send.rs

Compute `let fuel_unlimited = fuel_total == 0;` once after initialising `fuel_total`,
then gate all fuel logic behind `if !fuel_unlimited`:

- **Decrements** — skip all `fuel_remaining.saturating_sub(...)` calls.
- **Exhaustion checks** — skip all `if fuel_remaining == 0` guards.
- **Verbose diagnostics** — skip fuel-tagged diagnostic messages.
- **Continuation prompt prefix** — when unlimited, emit:
  `[reengaged via {fallback_tool}. call_user(<message>) to end turn.]\n{prompt}`
  (no fuel numbers).
- **Hook payloads** — omit `fuel_remaining` and `fuel_total` keys entirely from all
  four hook data objects (`pre_api_tools`, `pre_api_request`, `pre_agentic_loop`,
  `post_tool_batch`).
- **`apply_hook_overrides`** — skip fuel override logic when unlimited (hook `fuel`/
  `fuel_delta` keys are silently ignored).

### Tests

Add a test confirming that `fuel = 0` does not inject fuel strings into the
continuation prompt prefix.
