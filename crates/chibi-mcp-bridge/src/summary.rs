//! LLM-powered tool summary generation via ratatoskr.
//!
//! Generates concise one-sentence summaries of MCP tool descriptions,
//! suitable for inclusion in an LLM's tool listing.

use ratatoskr::{ChatOptions, Message, ModelGateway, Ratatoskr};

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
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let gateway = Ratatoskr::builder().openrouter(api_key).build()?;

    let schema_str = serde_json::to_string_pretty(schema).unwrap_or_default();
    let prompt = format!(
        "Compress this MCP tool description into a single concise sentence \
         suitable for an LLM tool listing. Include key parameters.\n\n\
         Tool: {tool_name}\n\
         Description: {description}\n\
         Schema: {schema_str}"
    );

    let messages = vec![Message::user(prompt)];
    let options = ChatOptions::new(model).max_tokens(150);

    let response = gateway.chat(&messages, None, &options).await?;
    Ok(response.content.trim().to_string())
}

/// Fill cache gaps by generating summaries for tools that aren't cached yet.
///
/// Aborts on first failure (e.g. missing API key) to avoid spamming errors.
/// Returns the number of newly generated summaries.

pub async fn fill_cache_gaps(
    cache: &std::sync::Arc<tokio::sync::Mutex<crate::cache::SummaryCache>>,
    tools: &[crate::protocol::ToolInfo],
    model: &str,
    api_key: Option<&str>,
) -> usize {
    let mut generated = 0;

    for tool in tools {
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

        // LLM call runs without holding the lock
        let result =
            generate_summary(model, &tool.name, &tool.description, &tool.parameters, api_key)
                .await;
        match result {
            Ok(summary) => {
                eprintln!(
                    "[mcp-bridge] generated summary for {}:{}: {}",
                    tool.server, tool.name, summary
                );
                let mut cache = cache.lock().await;
                cache.set(&tool.server, &tool.name, &tool.parameters, summary);
                generated += 1;
            }
            Err(e) => {
                eprintln!(
                    "[mcp-bridge] summary generation failed, aborting: {e}",
                );
                break;
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
