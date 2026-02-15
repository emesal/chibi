//! Model metadata retrieval and formatting.
//!
//! Two output formats:
//! - **TOML** ([`format_model_toml`]): For `models.toml` copy-paste (CLI `-m`/`-M`).
//! - **JSON** ([`format_model_json`]): Structured output for the `model_info` tool.
//!
//! Metadata is fetched via [`fetch_metadata`], which delegates to ratatoskr's
//! `ModelGateway::fetch_model_metadata()` (registry → cache → network).

use ratatoskr::{
    EmbeddedGateway, ModelGateway, ModelMetadata, ParameterAvailability, ParameterName,
};
use std::fmt::Write;
use std::io;

/// Fetch model metadata through the gateway (registry → cache → network).
pub async fn fetch_metadata(gateway: &EmbeddedGateway, model: &str) -> io::Result<ModelMetadata> {
    gateway
        .fetch_model_metadata(model)
        .await
        .map_err(|e| io::Error::other(format!("failed to fetch metadata for '{}': {}", model, e)))
}

/// Format model metadata as a JSON value for tool output.
///
/// Uses serde serialisation since `ModelMetadata` derives `Serialize`.
/// Thin wrapper to allow future customisation of the tool response shape.
pub fn format_model_json(metadata: &ModelMetadata) -> serde_json::Value {
    serde_json::to_value(metadata).expect("ModelMetadata serialisation should not fail")
}

/// Format model metadata as TOML for `models.toml`.
///
/// When `full` is false, emits only the settable fields.
/// When `full` is true, includes informational comments.
pub fn format_model_toml(metadata: &ModelMetadata, full: bool) -> String {
    let mut out = String::new();
    let id = &metadata.info.id;

    if full {
        write_header_comments(&mut out, metadata);
    }

    // [models."<id>"]
    writeln!(out, "[models.\"{}\"]", id).unwrap();

    // context_window is informational only (sourced from ratatoskr registry)
    if full && let Some(ctx) = metadata.info.context_window {
        writeln!(out, "# context_window: {}", ctx).unwrap();
    }

    // [models."<id>".api]
    let has_api_fields =
        metadata.max_output_tokens.is_some() || (full && has_parameter_comments(metadata));

    if has_api_fields {
        writeln!(out).unwrap();
        writeln!(out, "[models.\"{}\".api]", id).unwrap();

        if let Some(max) = metadata.max_output_tokens {
            writeln!(out, "max_tokens = {}", max).unwrap();
        }

        if full {
            write_parameter_comments(&mut out, metadata);
        }
    }

    out
}

/// Write header comments with provider, capabilities, and pricing info.
fn write_header_comments(out: &mut String, metadata: &ModelMetadata) {
    writeln!(out, "# {}", metadata.info.id).unwrap();
    writeln!(out, "# provider: {}", metadata.info.provider).unwrap();

    if !metadata.info.capabilities.is_empty() {
        let caps: Vec<&str> = metadata
            .info
            .capabilities
            .iter()
            .map(|c| format_capability(c))
            .collect();
        writeln!(out, "# capabilities: {}", caps.join(", ")).unwrap();
    }

    if let Some(ref pricing) = metadata.pricing {
        let prompt = pricing
            .prompt_cost_per_mtok
            .map(format_cost)
            .unwrap_or_else(|| "?".to_string());
        let completion = pricing
            .completion_cost_per_mtok
            .map(format_cost)
            .unwrap_or_else(|| "?".to_string());
        writeln!(
            out,
            "# pricing: {} / {} per MTok (prompt/completion)",
            prompt, completion
        )
        .unwrap();
    }

    writeln!(out).unwrap();
}

/// Check if there are parameter comments worth writing.
fn has_parameter_comments(metadata: &ModelMetadata) -> bool {
    metadata.parameters.iter().any(|(name, _)| {
        // Skip max_tokens — already emitted as a settable field
        !matches!(name, ParameterName::MaxTokens)
    })
}

/// Write parameter availability as TOML comments.
fn write_parameter_comments(out: &mut String, metadata: &ModelMetadata) {
    // Stable ordering: well-known parameters first, then custom
    let mut params: Vec<_> = metadata.parameters.iter().collect();
    params.sort_by_key(|(name, _)| name.as_str().to_string());

    for (name, avail) in params {
        // Skip max_tokens — already a settable field above
        if matches!(name, ParameterName::MaxTokens) {
            continue;
        }

        match avail {
            ParameterAvailability::Mutable { range } => {
                let range_str = match (range.min, range.max) {
                    (Some(min), Some(max)) => {
                        Some(format!("{} - {}", format_num(min), format_num(max)))
                    }
                    (Some(min), None) => Some(format!("min {}", format_num(min))),
                    (None, Some(max)) => Some(format!("max {}", format_num(max))),
                    (None, None) => None,
                };
                let default_str = range.default.map(|d| format!("default: {}", format_num(d)));

                match (range_str, default_str) {
                    (Some(r), Some(d)) => writeln!(out, "# {}: {} ({})", name, r, d).unwrap(),
                    (Some(r), None) => writeln!(out, "# {}: {}", name, r).unwrap(),
                    (None, Some(d)) => writeln!(out, "# {}: supported ({})", name, d).unwrap(),
                    (None, None) => writeln!(out, "# {}: supported", name).unwrap(),
                }
            }
            ParameterAvailability::ReadOnly { value } => {
                writeln!(out, "# {}: {} (read-only)", name, value).unwrap();
            }
            ParameterAvailability::Opaque => {
                writeln!(out, "# {}: supported", name).unwrap();
            }
            ParameterAvailability::Unsupported => {
                // Skip unsupported parameters
            }
            _ => {
                writeln!(out, "# {}: supported", name).unwrap();
            }
        }
    }
}

/// Format a capability variant as a display string.
fn format_capability(cap: &ratatoskr::ModelCapability) -> &'static str {
    match cap {
        ratatoskr::ModelCapability::Chat => "Chat",
        ratatoskr::ModelCapability::Generate => "Generate",
        ratatoskr::ModelCapability::Embed => "Embed",
        ratatoskr::ModelCapability::Nli => "NLI",
        ratatoskr::ModelCapability::Classify => "Classify",
        ratatoskr::ModelCapability::Stance => "Stance",
        _ => "Unknown",
    }
}

/// Format a cost value as dollars (e.g., "$3.00").
fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

/// Format a number, omitting trailing ".0" for integers.
fn format_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatoskr::{
        ModelCapability, ModelInfo, ModelMetadata, ParameterAvailability, ParameterName,
        ParameterRange, PricingInfo,
    };

    /// Build a representative test metadata entry.
    fn test_metadata() -> ModelMetadata {
        ModelMetadata::from_info(
            ModelInfo::new("anthropic/claude-sonnet-4", "openrouter")
                .with_capability(ModelCapability::Chat)
                .with_context_window(200000),
        )
        .with_max_output_tokens(16384)
        .with_pricing(PricingInfo {
            prompt_cost_per_mtok: Some(3.0),
            completion_cost_per_mtok: Some(15.0),
        })
        .with_parameter(
            ParameterName::Temperature,
            ParameterAvailability::Mutable {
                range: ParameterRange::new().min(0.0).max(1.0).default_value(1.0),
            },
        )
        .with_parameter(
            ParameterName::TopP,
            ParameterAvailability::Mutable {
                range: ParameterRange::new().min(0.0).max(1.0),
            },
        )
        .with_parameter(
            ParameterName::TopK,
            ParameterAvailability::Mutable {
                range: ParameterRange::new().min(1.0),
            },
        )
        .with_parameter(ParameterName::Reasoning, ParameterAvailability::Opaque)
    }

    #[test]
    fn minimal_output_contains_settable_fields() {
        let md = test_metadata();
        let out = format_model_toml(&md, false);

        assert!(out.contains("[models.\"anthropic/claude-sonnet-4\"]"));
        assert!(out.contains("max_tokens = 16384"));
        // No comments in minimal mode (context_window is informational, full-only)
        assert!(!out.contains("context_window"));
        assert!(!out.contains("# provider"));
        assert!(!out.contains("# pricing"));
        assert!(!out.contains("# temperature"));
    }

    #[test]
    fn full_output_contains_comments() {
        let md = test_metadata();
        let out = format_model_toml(&md, true);

        assert!(out.contains("# anthropic/claude-sonnet-4"));
        assert!(out.contains("# provider: openrouter"));
        assert!(out.contains("# capabilities: Chat"));
        assert!(out.contains("# pricing: $3.00 / $15.00 per MTok"));
        assert!(out.contains("# context_window: 200000"));
        assert!(out.contains("# temperature:"));
        assert!(out.contains("# top_p:"));
        assert!(out.contains("# top_k:"));
        assert!(out.contains("# reasoning: supported"));
    }

    #[test]
    fn minimal_without_optional_fields() {
        let md = ModelMetadata::from_info(ModelInfo::new("test/model", "test"));
        let out = format_model_toml(&md, false);

        assert!(out.contains("[models.\"test/model\"]"));
        // No context_window or api section
        assert!(!out.contains("context_window"));
        assert!(!out.contains("[models.\"test/model\".api]"));
    }

    #[test]
    fn unsupported_parameters_are_hidden() {
        let md = ModelMetadata::from_info(ModelInfo::new("test/model", "test"))
            .with_parameter(ParameterName::Seed, ParameterAvailability::Unsupported);
        let out = format_model_toml(&md, true);

        assert!(!out.contains("seed"));
    }

    #[test]
    fn read_only_parameter_shown_in_full() {
        let md = ModelMetadata::from_info(ModelInfo::new("test/model", "test")).with_parameter(
            ParameterName::Temperature,
            ParameterAvailability::ReadOnly {
                value: serde_json::json!(0.7),
            },
        );
        let out = format_model_toml(&md, true);

        assert!(out.contains("# temperature: 0.7 (read-only)"));
    }

    #[test]
    fn format_num_integers() {
        assert_eq!(format_num(1.0), "1");
        assert_eq!(format_num(0.0), "0");
        assert_eq!(format_num(200000.0), "200000");
    }

    #[test]
    fn format_num_floats() {
        assert_eq!(format_num(0.5), "0.5");
        assert_eq!(format_num(1.23), "1.23");
    }

    #[test]
    fn test_format_model_json() {
        let md = test_metadata();
        let json = format_model_json(&md);

        // Top-level structure
        assert!(json.is_object());
        assert!(json.get("info").is_some());
        assert!(json.get("parameters").is_some());
        assert!(json.get("pricing").is_some());

        // Model identity
        assert_eq!(json["info"]["id"], "anthropic/claude-sonnet-4");
        assert_eq!(json["info"]["provider"], "openrouter");
        assert_eq!(json["info"]["context_window"], 200000);

        // Max output tokens
        assert_eq!(json["max_output_tokens"], 16384);

        // Pricing
        assert_eq!(json["pricing"]["prompt_cost_per_mtok"], 3.0);
        assert_eq!(json["pricing"]["completion_cost_per_mtok"], 15.0);
    }

    #[test]
    fn test_format_model_json_minimal() {
        let md = ModelMetadata::from_info(ModelInfo::new("test/model", "test"));
        let json = format_model_json(&md);

        assert_eq!(json["info"]["id"], "test/model");
        assert!(json["max_output_tokens"].is_null());
        assert!(json["pricing"].is_null());
    }
}
