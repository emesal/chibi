use serde::{Deserialize, Serialize};

/// Tool info returned by list_tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub server: String,
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Incoming request from chibi-core
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    ListTools,
    CallTool {
        server: String,
        tool: String,
        args: serde_json::Value,
    },
    GetSchema {
        server: String,
        tool: String,
    },
}

/// Outgoing response to chibi-core
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Tools { ok: bool, tools: Vec<ToolInfo> },
    Schema { ok: bool, schema: serde_json::Value },
    Result { ok: bool, result: String },
    Error { ok: bool, error: String },
}

impl Response {
    pub fn ok_tools(tools: Vec<ToolInfo>) -> Self {
        Self::Tools { ok: true, tools }
    }

    pub fn ok_result(result: String) -> Self {
        Self::Result { ok: true, result }
    }

    pub fn ok_schema(schema: serde_json::Value) -> Self {
        Self::Schema { ok: true, schema }
    }

    pub fn error(msg: String) -> Self {
        Self::Error {
            ok: false,
            error: msg,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_list_tools_serialisation() {
        let req: Request = serde_json::from_str(r#"{"op": "list_tools"}"#).unwrap();
        assert!(matches!(req, Request::ListTools));
    }

    #[test]
    fn request_call_tool_serialisation() {
        let req: Request = serde_json::from_str(
            r#"{"op": "call_tool", "server": "serena", "tool": "find_symbol", "args": {"name": "foo"}}"#,
        ).unwrap();
        match req {
            Request::CallTool { server, tool, args } => {
                assert_eq!(server, "serena");
                assert_eq!(tool, "find_symbol");
                assert_eq!(args, json!({"name": "foo"}));
            }
            _ => panic!("expected CallTool"),
        }
    }

    #[test]
    fn request_get_schema_serialisation() {
        let req: Request = serde_json::from_str(
            r#"{"op": "get_schema", "server": "serena", "tool": "read_file"}"#,
        )
        .unwrap();
        match req {
            Request::GetSchema { server, tool } => {
                assert_eq!(server, "serena");
                assert_eq!(tool, "read_file");
            }
            _ => panic!("expected GetSchema"),
        }
    }

    #[test]
    fn response_ok_tools() {
        let resp = Response::ok_tools(vec![ToolInfo {
            server: "s".into(),
            name: "t".into(),
            description: "d".into(),
            parameters: json!({}),
        }]);
        let v: serde_json::Value = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["tools"][0]["server"], "s");
    }

    #[test]
    fn response_ok_result() {
        let resp = Response::ok_result("hello".into());
        let v: serde_json::Value = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"], "hello");
    }

    #[test]
    fn response_ok_schema() {
        let resp = Response::ok_schema(json!({"type": "object"}));
        let v: serde_json::Value = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["schema"]["type"], "object");
    }

    #[test]
    fn response_error() {
        let resp = Response::error("bad".into());
        let v: serde_json::Value = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "bad");
    }

    #[test]
    fn response_roundtrip_tools() {
        let resp = Response::ok_tools(vec![]);
        let json = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Response::Tools { ok: true, tools } if tools.is_empty()));
    }

    #[test]
    fn response_roundtrip_error() {
        let resp = Response::error("oops".into());
        let json = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Response::Error { ok: false, error } if error == "oops"));
    }
}
