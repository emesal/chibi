use crate::config::ServerConfig;
use crate::protocol::ToolInfo;

use rmcp::model::{CallToolRequestParams, ListToolsResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};

use std::collections::HashMap;

/// A connected MCP server with its discovered tools.
struct ManagedServer {
    service: RunningService<RoleClient, ()>,
    tools: Vec<rmcp::model::Tool>,
}

/// Manages the lifecycle of MCP server processes.
pub struct ServerManager {
    servers: HashMap<String, ManagedServer>,
}

impl ServerManager {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// Spawn an MCP server process, connect via stdio, and discover its tools.
    pub async fn start_server(
        &mut self,
        name: &str,
        config: &ServerConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args);

        let transport = TokioChildProcess::new(cmd)?;
        let service = ().serve(transport).await?;

        let ListToolsResult { tools, .. } = service.list_tools(Default::default()).await?;

        eprintln!(
            "[mcp-bridge] server '{name}': {} tools discovered",
            tools.len()
        );

        self.servers
            .insert(name.to_string(), ManagedServer { service, tools });

        Ok(())
    }

    /// Aggregate tool info from all connected servers.
    pub fn list_all_tools(&self) -> Vec<ToolInfo> {
        self.servers
            .iter()
            .flat_map(|(server_name, managed)| {
                managed.tools.iter().map(move |tool| ToolInfo {
                    server: server_name.clone(),
                    name: tool.name.to_string(),
                    description: tool.description.as_deref().unwrap_or("").to_string(),
                    parameters: serde_json::to_value(&*tool.input_schema)
                        .unwrap_or(serde_json::Value::Object(Default::default())),
                })
            })
            .collect()
    }

    /// Call a tool on a specific server, returning the text result.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let managed = self
            .servers
            .get(server)
            .ok_or_else(|| format!("unknown server: {server}"))?;

        let arguments = args.as_object().cloned();

        let result = managed
            .service
            .call_tool(CallToolRequestParams {
                name: tool.to_string().into(),
                arguments,
                meta: None,
                task: None,
            })
            .await?;

        if result.is_error == Some(true) {
            let text = extract_text(&result.content);
            return Err(format!("tool error: {text}").into());
        }

        Ok(extract_text(&result.content))
    }

    /// Get the full input schema for a specific tool.
    pub fn get_schema(
        &self,
        server: &str,
        tool: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let managed = self
            .servers
            .get(server)
            .ok_or_else(|| format!("unknown server: {server}"))?;

        let mcp_tool = managed
            .tools
            .iter()
            .find(|t| t.name.as_ref() == tool)
            .ok_or_else(|| format!("unknown tool: {tool}"))?;

        Ok(serde_json::to_value(&*mcp_tool.input_schema)?)
    }
}

/// Extract text content from MCP response content blocks.
fn extract_text(content: &[rmcp::model::Content]) -> String {
    content
        .iter()
        .filter_map(|c| c.raw.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}
