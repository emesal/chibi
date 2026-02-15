//! LLM-powered tool summary generation via ratatoskr.
//!
//! Generates concise one-sentence summaries of MCP tool descriptions,
//! suitable for inclusion in an LLM's tool listing.

use ratatoskr::{ChatOptions, Message, ModelGateway, Ratatoskr};

/// Generate a concise summary of an MCP tool using an LLM.
///
/// The summary is a single sentence describing what the tool does and
/// its key parameters, suitable for an LLM tool listing.
pub async fn generate_summary(
    model: &str,
    tool_name: &str,
    description: &str,
    schema: &serde_json::Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let gateway = Ratatoskr::builder().openrouter(None::<String>).build()?;

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
/// Returns the number of newly generated summaries.
pub async fn fill_cache_gaps(
    cache: &mut crate::cache::SummaryCache,
    tools: &[crate::protocol::ToolInfo],
    model: &str,
) -> usize {
    let mut generated = 0;

    for tool in tools {
        if cache.get(&tool.server, &tool.name, &tool.parameters).is_some() {
            continue;
        }

        match generate_summary(model, &tool.name, &tool.description, &tool.parameters).await {
            Ok(summary) => {
                eprintln!(
                    "[mcp-bridge] generated summary for {}:{}: {}",
                    tool.server, tool.name, summary
                );
                cache.set(&tool.server, &tool.name, &tool.parameters, summary);
                generated += 1;
            }
            Err(e) => {
                eprintln!(
                    "[mcp-bridge] failed to generate summary for {}:{}: {e}",
                    tool.server, tool.name
                );
            }
        }
    }

    if generated > 0 {
        if let Err(e) = cache.save() {
            eprintln!("[mcp-bridge] failed to save summary cache: {e}");
        }
    }

    generated
}
