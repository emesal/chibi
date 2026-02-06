//! Model metadata formatting for CLI output.
//!
//! Formats [`ModelMetadata`] from ratatoskr's registry as TOML suitable for
//! copy-pasting into `models.toml`. Two modes:
//!
//! - **Minimal**: Only settable fields (`context_window`, `max_tokens`).
//! - **Full**: Adds comments for provider, capabilities, pricing, and parameter ranges.

use ratatoskr::{ModelMetadata, ModelRegistry, ParameterAvailability, ParameterName};
use std::fmt::Write;

/// Look up a model in the embedded registry and format as TOML.
///
/// Returns `None` if the model is not found.
pub fn lookup_and_format(model: &str, full: bool) -> Option<String> {
    let registry = ModelRegistry::with_embedded_seed();
    registry.get(model).map(|md| format_model_toml(md, full))
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

    if let Some(ctx) = metadata.info.context_window {
        writeln!(out, "context_window = {}", ctx).unwrap();
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
                    (None, Some(d)) => {
                        writeln!(out, "# {}: supported ({})", name, d).unwrap()
                    }
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
        assert!(out.contains("context_window = 200000"));
        assert!(out.contains("max_tokens = 16384"));
        // No comments in minimal mode
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
}
