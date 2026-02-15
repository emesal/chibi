use crate::protocol::{Request, Response};
use crate::server::ServerManager;

/// Dispatches incoming requests to the appropriate server manager method.
pub struct Bridge {
    pub server_manager: ServerManager,
}

impl Bridge {
    pub async fn handle_request(&self, req: Request) -> Response {
        match req {
            Request::ListTools => {
                Response::ok_tools(self.server_manager.list_all_tools())
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
