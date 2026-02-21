//! LLM-powered tool summary generation via ratatoskr.
//!
//! Generates concise one-sentence summaries of MCP tool descriptions,
//! suitable for inclusion in an LLM's tool listing.
//!
//! Transient failures (rate limits, network errors) are retried with
//! exponential backoff, honouring `Retry-After` hints when available.
//! Permanent failures (auth, model not found) abort the batch immediately.

use std::time::Duration;

use ratatoskr::{ChatOptions, Message, ModelGateway, Ratatoskr, RatatoskrError};

/// Maximum number of attempts per tool summary before giving up.
const MAX_ATTEMPTS: u32 = 3;

/// Initial backoff delay for transient errors (doubled each attempt).
const BACKOFF_BASE: Duration = Duration::from_secs(5);

/// Upper bound on backoff delay regardless of doubling or `Retry-After`.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Generate a concise summary of an MCP tool using an LLM.
///
/// The summary is a single sentence describing what the tool does and
/// its key parameters, suitable for an LLM tool listing.
/// Uses the OpenRouter API key from chibi's config.toml when available.
pub async fn generate_summary(
    model: &str,
    tool_name: &str,
    description: &str,
    schema: &serde_json::Value,
    api_key: Option<&str>,
) -> Result<String, RatatoskrError> {
    let gateway = Ratatoskr::builder()
        .openrouter(api_key)
        .build()
        .map_err(|e| RatatoskrError::Configuration(e.to_string()))?;

    let schema_str = serde_json::to_string_pretty(schema).unwrap_or_default();
    let prompt = format!(
        "Compress this MCP tool description into a single concise sentence \
         suitable for an LLM tool listing. Include key parameters.\n\n\
         Tool: {tool_name}\n\
         Description: {description}\n\
         Schema: {schema_str}"
    );

    let messages = vec![Message::user(prompt)];
    // Reasoning models (e.g. R1) use part of the budget for <think> chains,
    // so we need headroom beyond the ~1 sentence we actually want back.
    let options = ChatOptions::new(model).max_tokens(300);

    let response = gateway.chat(&messages, None, &options).await?;
    Ok(response.content.trim().to_string())
}

/// Fill cache gaps by generating summaries for tools that aren't cached yet.
///
/// Transient errors (rate limiting, network) are retried with exponential
/// backoff up to [`MAX_ATTEMPTS`] times, honouring `Retry-After` hints.
/// Permanent errors (auth failure, model not found) abort the batch — no
/// point attempting further tools if the API key or model is wrong.
///
/// Returns the number of newly generated summaries.
pub async fn fill_cache_gaps(
    cache: &std::sync::Arc<tokio::sync::Mutex<crate::cache::SummaryCache>>,
    tools: &[crate::protocol::ToolInfo],
    model: &str,
    api_key: Option<&str>,
) -> usize {
    let mut generated = 0;

    'tools: for tool in tools {
        // Lock briefly to check cache
        {
            let cache = cache.lock().await;
            if cache
                .get(&tool.server, &tool.name, &tool.parameters)
                .is_some()
            {
                continue;
            }
        }

        // Retry loop for transient failures
        let mut backoff = BACKOFF_BASE;
        for attempt in 1..=MAX_ATTEMPTS {
            let result = generate_summary(
                model,
                &tool.name,
                &tool.description,
                &tool.parameters,
                api_key,
            )
            .await;

            match result {
                Ok(summary) if !summary.is_empty() => {
                    eprintln!(
                        "[mcp-bridge] generated summary for {}:{}: {}",
                        tool.server, tool.name, summary
                    );
                    let mut cache = cache.lock().await;
                    cache.set(&tool.server, &tool.name, &tool.parameters, summary);
                    generated += 1;
                    continue 'tools;
                }
                Ok(_) => {
                    eprintln!(
                        "[mcp-bridge] empty summary for {}:{}, skipping",
                        tool.server, tool.name
                    );
                    continue 'tools;
                }
                Err(ref e) if e.is_transient() => {
                    if attempt == MAX_ATTEMPTS {
                        eprintln!(
                            "[mcp-bridge] summary failed for {}:{} after {MAX_ATTEMPTS} attempts ({e}), skipping",
                            tool.server, tool.name
                        );
                        continue 'tools;
                    }
                    // Honour Retry-After if provided, otherwise use backoff.
                    let wait = e.retry_after().unwrap_or(backoff).min(BACKOFF_MAX);
                    eprintln!(
                        "[mcp-bridge] transient error for {}:{} (attempt {attempt}/{MAX_ATTEMPTS}): {e} — retrying in {wait:?}",
                        tool.server, tool.name
                    );
                    tokio::time::sleep(wait).await;
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                }
                Err(e) => {
                    // Permanent error: no point continuing with other tools.
                    eprintln!(
                        "[mcp-bridge] summary generation aborted: {e}"
                    );
                    break 'tools;
                }
            }
        }
    }

    if generated > 0 {
        let cache = cache.lock().await;
        if let Err(e) = cache.save() {
            eprintln!("[mcp-bridge] failed to save summary cache: {e}");
        }
    }

    generated
}
