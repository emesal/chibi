# Ratatoskr Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace chibi's HTTP/SSE layer with ratatoskr's `ModelGateway`, keeping chibi's agentic loop intact.

**Architecture:** Ratatoskr owns LLM communication (HTTP, SSE parsing, tool call reconstruction). Chibi owns the agentic loop, hooks, tool execution, and context management. A thin conversion layer bridges chibi's types to ratatoskr's types.

**Tech Stack:** Rust, ratatoskr (local path dependency), futures-util for stream handling.

---

## Progress

| Task | Status | Commit |
|------|--------|--------|
| 1. Add ratatoskr dependency | ✅ Done | `927b39a` |
| 2. Create gateway module | ✅ Done | `d5732f8` |
| 3. Replace collect_streaming_response | ✅ Done | `e4caf04` |
| 4. Clean up unused code | ✅ Done | pending |
| 5. Integration testing | ✅ Done | n/a (found bug #106) |
| 6. Documentation update | ✅ Done | pending |

**All tests passing:** 275 tests ✅

---

## Task 4: Clean Up Unused Code

**Files:**
- Modify: `crates/chibi-core/src/llm.rs`
- Possibly modify: `crates/chibi-core/src/api/request.rs`

**Step 1: Check what's still used in llm.rs**

Run: `grep -r "llm::" crates/chibi-core/src/`

The `send_streaming_request` function should no longer be called. `ToolCallAccumulator` is still used.

**Step 2: Simplify llm.rs**

Remove the `send_streaming_request` function and any related HTTP code. Keep only:

```rust
//! LLM types used during streaming response accumulation.

/// Accumulated tool call data during streaming
#[derive(Default)]
pub struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
```

**Step 3: Check if request.rs build_request_body is still needed**

Run: `grep -r "build_request_body" crates/chibi-core/src/`

It's still used for:
- Logging (`log_request_if_enabled`)
- `pre_api_request` hook data

So keep `build_request_body` but it's now only for logging/hooks, not for actual API calls.

**Step 4: Run tests**

Run: `cargo test -p chibi-core`
Expected: All tests pass

**Step 5: Commit**

```bash
git add crates/chibi-core/src/llm.rs
git commit -m "$(cat <<'EOF'
chore: remove unused HTTP code after ratatoskr integration

The send_streaming_request function is replaced by ratatoskr.
Keeps ToolCallAccumulator which is still used for streaming accumulation.
EOF
)"
```

---

## Task 5: Integration Testing

**Step 1: Manual integration test**

Run chibi with a real prompt to verify the integration works end-to-end:

```bash
cargo run -- "What is 2+2?"
```

Expected: Should get a response from the LLM via ratatoskr.

**Step 2: Test tool calling**

```bash
cargo run -- "What time is it?"
```

(Assuming a time tool exists, or test with any tool-using prompt)

**Step 3: Run full test suite**

Run: `just pre-push`
Expected: All formatting, linting, and tests pass

**Step 4: Final commit (if any fixes needed)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
test: verify ratatoskr integration works end-to-end
EOF
)"
```

---

## Task 6: Documentation Update

**Files:**
- Modify: `CLAUDE.md` (if architecture section needs update)

**Step 1: Update architecture description**

If the CLAUDE.md mentions `llm.rs` or the HTTP layer specifically, update it to reflect that ratatoskr now handles LLM communication.

**Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "$(cat <<'EOF'
docs: update architecture to reflect ratatoskr integration
EOF
)"
```

---

## Summary

**Key invariants preserved:**
- Agentic loop unchanged in `send.rs`
- Hook orchestration unchanged
- Tool execution unchanged
- Context/transcript management unchanged
- `ResponseSink` interface unchanged

**What changed:**
- HTTP request → `gateway.chat_stream()`
- SSE parsing → `ChatEvent` stream consumption
- `ToolCallAccumulator` populated from `ChatEvent::ToolCallStart/Delta` instead of raw JSON
- Added `crates/chibi-core/src/gateway.rs` with type conversion functions
