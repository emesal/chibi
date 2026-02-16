use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cache::SummaryCache;
use crate::protocol::{Request, Response};
use crate::server::ServerManager;

/// Dispatches incoming requests to the appropriate server manager method.
///
/// When `summary_cache` is `Some`, tool descriptions in `ListTools` responses
/// are replaced with cached LLM-generated summaries (falling back to originals
/// for uncached tools). `None` disables summary substitution entirely.
pub struct Bridge {
    pub server_manager: ServerManager,
    pub summary_cache: Option<Arc<Mutex<SummaryCache>>>,
}

impl Bridge {
    pub async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::ListTools => {
                let mut tools = self.server_manager.list_all_tools();
                if let Some(cache) = &self.summary_cache {
                    let cache = cache.lock().await;
                    for tool in &mut tools {
                        if let Some(summary) = cache.get(&tool.server, &tool.name, &tool.parameters)
                        {
                            tool.description = summary.to_string();
                        }
                    }
                }
                Response::ok_tools(tools)
            }
            Request::CallTool { server, tool, args } => {
                match self.server_manager.call_tool(&server, &tool, &args).await {
                    Ok(result) => Response::ok_result(result),
                    Err(e) => Response::error(e.to_string()),
                }
            }
            Request::GetSchema { server, tool } => {
                match self.server_manager.get_schema(&server, &tool) {
                    Ok(schema) => Response::ok_schema(schema),
                    Err(e) => Response::error(e.to_string()),
                }
            }
        }
    }
}
