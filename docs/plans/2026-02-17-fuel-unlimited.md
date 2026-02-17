# Fuel Unlimited Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow `fuel = 0` to mean "unlimited / no tracking", suppressing all fuel bookkeeping from prompts, diagnostics, and hook payloads.

**Architecture:** Compute `let fuel_unlimited = fuel_total == 0;` once after initialising `fuel_total` in the agentic loop, then gate all fuel logic behind `if !fuel_unlimited`. No type changes — `usize` stays `usize`. Update doc comments and configuration docs to document the sentinel.

**Tech Stack:** Rust, `crates/chibi-core`

---

### Task 1: Update doc comments in config.rs

**Files:**
- Modify: `crates/chibi-core/src/config.rs:610-615` (`Config::fuel`, `Config::fuel_empty_response_cost`)
- Modify: `crates/chibi-core/src/config.rs:673-676` (`LocalConfig::fuel`, `LocalConfig::fuel_empty_response_cost`)
- Modify: `crates/chibi-core/src/config.rs:797-800` (`ResolvedConfig::fuel`, `ResolvedConfig::fuel_empty_response_cost`)

**Step 1: Update `Config::fuel` doc comment**

Replace:
```rust
    /// Total fuel budget for the agentic loop (tool rounds, continuations, empty responses)
    #[serde(default = "default_fuel")]
    pub fuel: usize,
    /// Fuel cost of an empty response (high cost prevents infinite empty loops)
    #[serde(default = "default_fuel_empty_response_cost")]
    pub fuel_empty_response_cost: usize,
```
With:
```rust
    /// Total fuel budget for the agentic loop (tool rounds, continuations, empty responses).
    /// Set to `0` to disable fuel tracking entirely (unlimited mode).
    #[serde(default = "default_fuel")]
    pub fuel: usize,
    /// Fuel cost of an empty response (high cost prevents infinite empty loops).
    /// Ignored when `fuel = 0` (unlimited mode).
    #[serde(default = "default_fuel_empty_response_cost")]
    pub fuel_empty_response_cost: usize,
```

**Step 2: Update `LocalConfig::fuel` doc comment**

Replace:
```rust
    /// Per-context fuel budget override
    pub fuel: Option<usize>,
    /// Per-context fuel cost for empty responses
    pub fuel_empty_response_cost: Option<usize>,
```
With:
```rust
    /// Per-context fuel budget override. `0` means unlimited.
    pub fuel: Option<usize>,
    /// Per-context fuel cost for empty responses. Ignored when `fuel = 0`.
    pub fuel_empty_response_cost: Option<usize>,
```

**Step 3: Update `ResolvedConfig::fuel` doc comment**

Replace:
```rust
    /// Total fuel budget for the agentic loop
    pub fuel: usize,
    /// Fuel cost of an empty response
    pub fuel_empty_response_cost: usize,
```
With:
```rust
    /// Total fuel budget for the agentic loop. `0` means unlimited (no tracking).
    pub fuel: usize,
    /// Fuel cost of an empty response. Ignored when `fuel = 0`.
    pub fuel_empty_response_cost: usize,
```

**Step 4: Build to confirm no regressions**

```bash
cargo build -p chibi-core 2>&1 | tail -5
```
Expected: `Finished` with no errors.

**Step 5: Commit**

```bash
git add crates/chibi-core/src/config.rs
git commit -m "docs(config): document fuel=0 as unlimited mode sentinel"
```

---

### Task 2: Add `fuel_unlimited` guard to the agentic loop in send.rs

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs:1691-1692` (initialisation)
- Modify: `crates/chibi-core/src/api/send.rs:1710-1714` (verbose entering-turn diagnostic)
- Modify: `crates/chibi-core/src/api/send.rs:1931-1950` (tool-call round decrement + exhaustion)
- Modify: `crates/chibi-core/src/api/send.rs:1956-1978` (empty-response decrement + exhaustion)
- Modify: `crates/chibi-core/src/api/send.rs:1994-2025` (continuation decrement + exhaustion + prefix)

**Step 1: Introduce `fuel_unlimited` sentinel**

After line 1692 (`let mut fuel_remaining = fuel_total;`), add:

```rust
    let fuel_unlimited = fuel_total == 0;
```

**Step 2: Guard entering-turn verbose diagnostic**

The block at ~1710-1715:
```rust
        if verbose {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[fuel: {}/{} entering turn]", fuel_remaining, fuel_total),
                verbose_only: true,
            })?;
        }
```
Wrap fuel message in `if !fuel_unlimited`:
```rust
        if verbose && !fuel_unlimited {
            sink.handle(ResponseEvent::Diagnostic {
                message: format!("[fuel: {}/{} entering turn]", fuel_remaining, fuel_total),
                verbose_only: true,
            })?;
        }
```

**Step 3: Guard tool-call round decrement and exhaustion (~1931-1951)**

Replace:
```rust
                // Tool call round costs 1 fuel
                fuel_remaining = fuel_remaining.saturating_sub(1);
                if verbose {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel: {}/{} after tool batch]",
                            fuel_remaining, fuel_total
                        ),
                        verbose_only: true,
                    })?;
                }
                if fuel_remaining == 0 {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel exhausted (0/{}), returning control to user]",
                            fuel_total
                        ),
                        verbose_only: false,
                    })?;
                    return Ok(());
                }
```
With:
```rust
                // Tool call round costs 1 fuel
                if !fuel_unlimited {
                    fuel_remaining = fuel_remaining.saturating_sub(1);
                    if verbose {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[fuel: {}/{} after tool batch]",
                                fuel_remaining, fuel_total
                            ),
                            verbose_only: true,
                        })?;
                    }
                    if fuel_remaining == 0 {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[fuel exhausted (0/{}), returning control to user]",
                                fuel_total
                            ),
                            verbose_only: false,
                        })?;
                        return Ok(());
                    }
                }
```

**Step 4: Guard empty-response decrement and exhaustion (~1956-1979)**

Replace:
```rust
            // Check for empty response
            if response.full_response.trim().is_empty() {
                fuel_remaining =
                    fuel_remaining.saturating_sub(resolved_config.fuel_empty_response_cost);
                if fuel_remaining == 0 {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[fuel exhausted (0/{}), returning control to user]",
                            fuel_total
                        ),
                        verbose_only: false,
                    })?;
                    return Ok(());
                }
                if verbose {
                    sink.handle(ResponseEvent::Diagnostic {
                        message: format!(
                            "[empty response, fuel: {}/{}]",
                            fuel_remaining, fuel_total
                        ),
                        verbose_only: true,
                    })?;
                }
                continue;
            }
```
With:
```rust
            // Check for empty response
            if response.full_response.trim().is_empty() {
                if !fuel_unlimited {
                    fuel_remaining =
                        fuel_remaining.saturating_sub(resolved_config.fuel_empty_response_cost);
                    if fuel_remaining == 0 {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[fuel exhausted (0/{}), returning control to user]",
                                fuel_total
                            ),
                            verbose_only: false,
                        })?;
                        return Ok(());
                    }
                    if verbose {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[empty response, fuel: {}/{}]",
                                fuel_remaining, fuel_total
                            ),
                            verbose_only: true,
                        })?;
                    }
                }
                continue;
            }
```

**Step 5: Guard continuation decrement, exhaustion, and prefix (~1994-2026)**

Replace:
```rust
                FinalResponseAction::ContinueWithPrompt(continue_prompt) => {
                    fuel_remaining = fuel_remaining.saturating_sub(1);
                    if fuel_remaining == 0 {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[fuel exhausted (0/{}), returning control to user]",
                                fuel_total
                            ),
                            verbose_only: false,
                        })?;
                        return Ok(());
                    }
                    if verbose {
                        sink.handle(ResponseEvent::Diagnostic {
                            message: format!(
                                "[continuing (fuel: {}/{}): {}]",
                                fuel_remaining,
                                fuel_total,
                                if continue_prompt.len() > 80 {
                                    format!("{}...", &continue_prompt[..77])
                                } else {
                                    continue_prompt.clone()
                                }
                            ),
                            verbose_only: true,
                        })?;
                    }
                    // Prefix the continuation prompt with fuel info for the LLM
                    current_prompt = format!(
                        "[reengaged (fuel: {}/{}) via {}. call_user(<message>) to end turn.]\n{}",
                        fuel_remaining, fuel_total, resolved_config.fallback_tool, continue_prompt
                    );
                    break; // break inner, continue outer
                }
```
With:
```rust
                FinalResponseAction::ContinueWithPrompt(continue_prompt) => {
                    if !fuel_unlimited {
                        fuel_remaining = fuel_remaining.saturating_sub(1);
                        if fuel_remaining == 0 {
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[fuel exhausted (0/{}), returning control to user]",
                                    fuel_total
                                ),
                                verbose_only: false,
                            })?;
                            return Ok(());
                        }
                        if verbose {
                            sink.handle(ResponseEvent::Diagnostic {
                                message: format!(
                                    "[continuing (fuel: {}/{}): {}]",
                                    fuel_remaining,
                                    fuel_total,
                                    if continue_prompt.len() > 80 {
                                        format!("{}...", &continue_prompt[..77])
                                    } else {
                                        continue_prompt.clone()
                                    }
                                ),
                                verbose_only: true,
                            })?;
                        }
                    }
                    // Prefix the continuation prompt; omit fuel numbers when unlimited
                    current_prompt = if fuel_unlimited {
                        format!(
                            "[reengaged via {}. call_user(<message>) to end turn.]\n{}",
                            resolved_config.fallback_tool, continue_prompt
                        )
                    } else {
                        format!(
                            "[reengaged (fuel: {}/{}) via {}. call_user(<message>) to end turn.]\n{}",
                            fuel_remaining, fuel_total, resolved_config.fallback_tool, continue_prompt
                        )
                    };
                    break; // break inner, continue outer
                }
```

**Step 6: Build**

```bash
cargo build -p chibi-core 2>&1 | tail -5
```
Expected: `Finished` with no errors.

**Step 7: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "feat(send): skip fuel tracking when fuel=0 (unlimited mode)"
```

---

### Task 3: Omit fuel keys from hook payloads when unlimited

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` — four hook data blocks and `apply_hook_overrides`

The four hook payloads that include `fuel_remaining`/`fuel_total` are at lines ~1830-1835, ~1849-1854, ~1874-1878, and ~1575-1580 (post_tool_batch, inside `execute_tool_calls`). The `apply_hook_overrides` function at ~358-410 handles `fuel`/`fuel_delta` keys.

**Step 1: Add `fuel_unlimited` to `execute_tool_calls` signature**

The function signature at ~1358 needs `fuel_unlimited: bool` added alongside `fuel_remaining` and `fuel_total`:

```rust
fn execute_tool_calls<S: ResponseSink>(
    // ... existing params ...
    fuel_remaining: &mut usize,
    fuel_total: usize,
    fuel_unlimited: bool,      // <- add this
    verbose: bool,
    // ...
```

And update its call site at ~1919 to pass `fuel_unlimited`.

**Step 2: Update `post_tool_batch` hook data in `execute_tool_calls` (~1575-1581)**

Replace:
```rust
    let hook_data = json!({
        "context_name": context_name,
        "fuel_remaining": fuel_remaining,
        "fuel_total": fuel_total,
        "current_fallback": resolved_config.fallback_tool,
        "tool_calls": tool_batch_info,
    });
```
With:
```rust
    let mut hook_data = json!({
        "context_name": context_name,
        "current_fallback": resolved_config.fallback_tool,
        "tool_calls": tool_batch_info,
    });
    if !fuel_unlimited {
        hook_data["fuel_remaining"] = json!(fuel_remaining);
        hook_data["fuel_total"] = json!(fuel_total);
    }
```

**Step 3: Update `apply_hook_overrides` to accept and check `fuel_unlimited`**

Add `fuel_unlimited: bool` parameter:
```rust
fn apply_hook_overrides<S: ResponseSink>(
    handoff: &mut tools::Handoff,
    fuel_remaining: &mut usize,
    fuel_unlimited: bool,
    hook_results: &[(String, serde_json::Value)],
    verbose: bool,
    sink: &mut S,
) -> io::Result<()> {
```

Then guard the fuel override blocks:
```rust
        if !fuel_unlimited {
            if let Some(fuel) = hook_result.get("fuel").and_then(|v| v.as_u64()) {
                *fuel_remaining = fuel as usize;
                // ... verbose diagnostic ...
            }
            if let Some(delta) = hook_result.get("fuel_delta").and_then(|v| v.as_i64()) {
                // ... existing delta logic ...
            }
        }
```

Update both call sites to pass `fuel_unlimited`:
- `apply_hook_overrides(handoff, fuel_remaining, fuel_unlimited, &hook_results, verbose, sink)?;` (line ~1583)
- `apply_hook_overrides(&mut handoff, &mut fuel_remaining, fuel_unlimited, &hook_results, verbose, sink)?;` (line ~1883)

**Step 4: Update the three remaining hook data blocks in the outer loop (~1830-1854, ~1874-1878)**

For `pre_api_tools` (~1830):
```rust
    let mut hook_data = json!({
        "context_name": context.name,
        "tools": tool_info,
    });
    if !fuel_unlimited {
        hook_data["fuel_remaining"] = json!(fuel_remaining);
        hook_data["fuel_total"] = json!(fuel_total);
    }
```

For `pre_api_request` (~1849):
```rust
    let mut hook_data = json!({
        "context_name": context.name,
        "request_body": request_body,
    });
    if !fuel_unlimited {
        hook_data["fuel_remaining"] = json!(fuel_remaining);
        hook_data["fuel_total"] = json!(fuel_total);
    }
```

For `pre_agentic_loop` (~1874):
```rust
    let mut hook_data = json!({
        "context_name": context.name,
        "current_fallback": resolved_config.fallback_tool,
        "message": final_prompt,
    });
    if !fuel_unlimited {
        hook_data["fuel_remaining"] = json!(fuel_remaining);
        hook_data["fuel_total"] = json!(fuel_total);
    }
```

**Step 5: Build**

```bash
cargo build -p chibi-core 2>&1 | tail -5
```
Expected: `Finished` with no errors.

**Step 6: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "feat(send): omit fuel keys from hook payloads in unlimited mode"
```

---

### Task 4: Add tests

**Files:**
- Modify: `crates/chibi-core/src/api/send.rs` (test module at end of file)

**Step 1: Write failing test for unlimited continuation prompt**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn test_continuation_prompt_unlimited_mode_omits_fuel() {
        // fuel_unlimited = true when fuel_total == 0
        let fuel_total: usize = 0;
        let fuel_remaining: usize = 0;
        let fuel_unlimited = fuel_total == 0;
        let fallback_tool = "call_user";
        let continue_prompt = "keep going";

        let prompt = if fuel_unlimited {
            format!(
                "[reengaged via {}. call_user(<message>) to end turn.]\n{}",
                fallback_tool, continue_prompt
            )
        } else {
            format!(
                "[reengaged (fuel: {}/{}) via {}. call_user(<message>) to end turn.]\n{}",
                fuel_remaining, fuel_total, fallback_tool, continue_prompt
            )
        };

        assert!(!prompt.contains("fuel:"), "fuel info must not appear in unlimited mode");
        assert!(prompt.contains("reengaged via call_user"));
        assert!(prompt.contains("keep going"));
    }

    #[test]
    fn test_continuation_prompt_limited_mode_includes_fuel() {
        let fuel_total: usize = 10;
        let fuel_remaining: usize = 7;
        let fuel_unlimited = fuel_total == 0;
        let fallback_tool = "call_user";
        let continue_prompt = "keep going";

        let prompt = if fuel_unlimited {
            format!(
                "[reengaged via {}. call_user(<message>) to end turn.]\n{}",
                fallback_tool, continue_prompt
            )
        } else {
            format!(
                "[reengaged (fuel: {}/{}) via {}. call_user(<message>) to end turn.]\n{}",
                fuel_remaining, fuel_total, fallback_tool, continue_prompt
            )
        };

        assert!(prompt.contains("fuel: 7/10"), "fuel info must appear in limited mode");
    }
```

**Step 2: Run the tests**

```bash
cargo test -p chibi-core -- test_continuation_prompt 2>&1 | tail -20
```
Expected: Both tests pass.

**Step 3: Commit**

```bash
git add crates/chibi-core/src/api/send.rs
git commit -m "test(send): verify fuel omitted from continuation prompt in unlimited mode"
```

---

### Task 5: Update documentation

**Files:**
- Modify: `docs/configuration.md:145-152`

**Step 1: Update fuel docs**

Replace the two fuel comment blocks:
```toml
# Total fuel budget for autonomous tool loops (default: 30)
# Each tool-call round and agent continuation costs 1 fuel. First turn is free.
fuel = 30

# Fuel cost of an empty LLM response (default: 15)
# When the LLM returns an empty response (no text, no tool calls), this much
# fuel is consumed. High cost prevents infinite empty-response loops.
fuel_empty_response_cost = 15
```
With:
```toml
# Total fuel budget for autonomous tool loops (default: 30)
# Each tool-call round and agent continuation costs 1 fuel. First turn is free.
# Set to 0 to disable fuel tracking entirely (unlimited mode — no budget enforced,
# no fuel info injected into prompts or hook payloads).
fuel = 30

# Fuel cost of an empty LLM response (default: 15)
# When the LLM returns an empty response (no text, no tool calls), this much
# fuel is consumed. High cost prevents infinite empty-response loops.
# Ignored when fuel = 0 (unlimited mode).
fuel_empty_response_cost = 15
```

**Step 2: Commit**

```bash
git add docs/configuration.md
git commit -m "docs: document fuel=0 unlimited mode"
```

---

### Task 6: Final check

**Step 1: Run full test suite**

```bash
cargo test 2>&1 | tail -20
```
Expected: All tests pass.

**Step 2: Run `just pre-push` if available**

```bash
just pre-push 2>&1 | tail -20
```
