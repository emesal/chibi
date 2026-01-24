//! Low-level LLM API communication.
//!
//! This module handles HTTP requests to OpenRouter/compatible APIs,
//! streaming response parsing, and raw message formatting.

use crate::config::ResolvedConfig;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use std::io;

/// Accumulated tool call data during streaming
#[derive(Default)]
pub struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Send a streaming request to the LLM API and process the response
///
/// Returns the raw response bytes stream for parsing
pub async fn send_streaming_request(
    config: &ResolvedConfig,
    request_body: serde_json::Value,
) -> io::Result<reqwest::Response> {
    let client = Client::new();

    let response = client
        .post(&config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Failed to send request: {}", e)))?;

    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::other(format!(
            "API error ({}): {}",
            status, body
        )));
    }

    Ok(response)
}
