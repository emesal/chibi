//! Tests for state module.

use super::*;
use crate::config::{ApiParams, LocalConfig, ToolsConfig};
use crate::context::InboxEntry;
use crate::partition::StorageConfig;
use serde_json::json;
use tempfile::TempDir;

/// Create a test AppState with a temporary directory
fn create_test_app() -> (AppState, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("test-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false,
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };
    let app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();
    (app, temp_dir)
}

// === Path construction tests ===

#[test]
fn test_context_dir() {
    let (app, _temp) = create_test_app();
    let dir = app.context_dir("mycontext");
    assert!(dir.ends_with("contexts/mycontext"));
}

#[test]
fn test_context_file() {
    let (app, _temp) = create_test_app();
    let file = app.context_file("mycontext");
    assert!(file.ends_with("contexts/mycontext/context.jsonl"));
}

#[test]
fn test_todos_file() {
    let (app, _temp) = create_test_app();
    let file = app.todos_file("mycontext");
    assert!(file.ends_with("contexts/mycontext/todos.md"));
}

#[test]
fn test_goals_file() {
    let (app, _temp) = create_test_app();
    let file = app.goals_file("mycontext");
    assert!(file.ends_with("contexts/mycontext/goals.md"));
}

#[test]
fn test_inbox_file() {
    let (app, _temp) = create_test_app();
    let file = app.inbox_file("mycontext");
    assert!(file.ends_with("contexts/mycontext/inbox.jsonl"));
}

// === Context lifecycle tests ===

#[test]
fn test_get_or_create_context_creates_default() {
    let (app, _temp) = create_test_app();
    let context = app.get_or_create_context("default").unwrap();
    assert_eq!(context.name, "default");
    assert!(context.messages.is_empty());
}

#[test]
fn test_save_and_load_context() {
    let (app, _temp) = create_test_app();

    let context = Context {
        name: "test-context".to_string(),
        messages: vec![
            json!({"_id": "m1", "role": "user", "content": "Hello"}),
            json!({"_id": "m2", "role": "assistant", "content": "Hi there!"}),
        ],
        created_at: 1234567890,
        updated_at: 1234567891,
        summary: "Test summary".to_string(),
    };

    app.save_context(&context).unwrap();

    let loaded = app.load_context("test-context").unwrap();
    assert_eq!(loaded.name, "test-context");
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0]["content"].as_str().unwrap(), "Hello");
    assert_eq!(loaded.summary, "Test summary");
}

#[test]
fn test_add_message() {
    let (app, _temp) = create_test_app();
    let mut context = app.get_or_create_context("default").unwrap();

    assert!(context.messages.is_empty());

    app.add_message(&mut context, "user".to_string(), "Test message".to_string());

    assert_eq!(context.messages.len(), 1);
    assert_eq!(context.messages[0]["role"].as_str().unwrap(), "user");
    assert_eq!(
        context.messages[0]["content"].as_str().unwrap(),
        "Test message"
    );
    assert!(context.updated_at > 0);
}

#[test]
fn test_list_contexts_empty() {
    let (app, _temp) = create_test_app();
    let contexts = app.list_contexts();
    assert!(contexts.is_empty());
}

#[test]
fn test_list_contexts_with_contexts() {
    let (mut app, _temp) = create_test_app();

    // Create some contexts
    for name in &["alpha", "beta", "gamma"] {
        let context = Context {
            name: name.to_string(),
            messages: vec![],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();
    }

    // Sync state with filesystem (discovers new directories)
    app.sync_state_with_filesystem().unwrap();

    let contexts = app.list_contexts();
    assert_eq!(contexts.len(), 3);
    // Should be sorted
    assert_eq!(contexts[0], "alpha");
    assert_eq!(contexts[1], "beta");
    assert_eq!(contexts[2], "gamma");
}

#[test]
fn test_rename_context() {
    let (mut app, _temp) = create_test_app();

    // Create a context
    let context = Context {
        name: "old-name".to_string(),
        messages: vec![json!({"_id": "m1", "role": "user", "content": "Hello"})],
        created_at: 0,
        updated_at: 0,
        summary: String::new(),
    };
    app.save_context(&context).unwrap();

    // Rename
    app.rename_context("old-name", "new-name").unwrap();

    // Verify
    assert!(!app.context_dir("old-name").exists());
    assert!(app.context_dir("new-name").exists());

    let loaded = app.load_context("new-name").unwrap();
    assert_eq!(loaded.name, "new-name");
    assert_eq!(loaded.messages[0]["content"].as_str().unwrap(), "Hello");
}

#[test]
fn test_rename_nonexistent_context() {
    let (mut app, _temp) = create_test_app();
    let result = app.rename_context("nonexistent", "new-name");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not exist"));
}

#[test]
fn test_rename_to_existing_context() {
    let (mut app, _temp) = create_test_app();

    // Create both contexts
    for name in &["source", "target"] {
        let context = Context {
            name: name.to_string(),
            messages: vec![],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();
    }

    let result = app.rename_context("source", "target");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}

#[test]
fn test_destroy_context() {
    let (mut app, _temp) = create_test_app();

    // Create context to destroy
    let context = Context {
        name: "to-destroy".to_string(),
        messages: vec![],
        created_at: 0,
        updated_at: 0,
        summary: String::new(),
    };
    app.save_context(&context).unwrap();

    // Destroy
    let result = app.destroy_context("to-destroy").unwrap();
    assert!(result); // Destroyed successfully
    assert!(!app.context_dir("to-destroy").exists());
}

#[test]
fn test_destroy_nonexistent_context() {
    let (mut app, _temp) = create_test_app();
    let result = app.destroy_context("nonexistent").unwrap();
    assert!(!result); // Nothing to destroy
}

// NOTE: Tests for "destroy current context switches to previous" were removed
// in the stateless-core refactor. Session state (current/previous context)
// is now managed by the CLI layer, not chibi-core.

// === Token calculation tests ===

#[test]
fn test_calculate_token_count_empty() {
    let (app, _temp) = create_test_app();
    let count = app.calculate_token_count(&[]);
    assert_eq!(count, 0);
}

#[test]
fn test_calculate_token_count() {
    let (app, _temp) = create_test_app();
    let messages = vec![json!({"role": "user", "content": "Hello world!"})];
    let count = app.calculate_token_count(&messages);
    // Now based on serialized JSON length / 4, which includes keys and quotes
    assert!(count > 0);
}

#[test]
fn test_remaining_tokens() {
    let (app, _temp) = create_test_app();
    let messages = vec![json!({"role": "user", "content": "x".repeat(4000)})];
    let remaining = app.remaining_tokens(&messages);
    // 8000 - estimated tokens from serialized JSON
    assert!(remaining < 8000);
    assert!(remaining > 5000);
}

#[test]
fn test_should_warn() {
    let (app, _temp) = create_test_app();

    // Small message shouldn't warn
    let small_messages = vec![json!({"role": "user", "content": "Hello"})];
    assert!(!app.should_warn(&small_messages));

    // Large message should warn (above 75% of 8000 = 6000 tokens ≈ 24000 chars)
    let large_messages = vec![json!({"role": "user", "content": "x".repeat(30000)})];
    assert!(app.should_warn(&large_messages));
}

// === Todos/Goals tests ===

#[test]
fn test_todos_save_and_load() {
    let (app, _temp) = create_test_app();

    app.save_todos("default", "- [ ] Task 1\n- [x] Task 2")
        .unwrap();
    let loaded = app.load_todos("default").unwrap();
    assert_eq!(loaded, "- [ ] Task 1\n- [x] Task 2");
}

#[test]
fn test_todos_empty_returns_empty_string() {
    let (app, _temp) = create_test_app();
    let loaded = app.load_todos("nonexistent").unwrap();
    assert_eq!(loaded, "");
}

#[test]
fn test_goals_save_and_load() {
    let (app, _temp) = create_test_app();

    app.save_goals("default", "Build something awesome")
        .unwrap();
    let loaded = app.load_goals("default").unwrap();
    assert_eq!(loaded, "Build something awesome");
}

// === Local config tests ===

#[test]
fn test_local_config_default() {
    let (app, _temp) = create_test_app();
    let local = app.load_local_config("default").unwrap();
    assert!(local.model.is_none());
    assert!(local.username.is_none());
}

#[test]
fn test_local_config_save_and_load() {
    let (app, _temp) = create_test_app();

    let local = LocalConfig {
        model: Some("custom-model".to_string()),
        username: Some("alice".to_string()),
        auto_compact: Some(true),
        ..Default::default()
    };

    app.save_local_config("default", &local).unwrap();
    let loaded = app.load_local_config("default").unwrap();

    assert_eq!(loaded.model, Some("custom-model".to_string()));
    assert_eq!(loaded.username, Some("alice".to_string()));
    assert_eq!(loaded.auto_compact, Some(true));
}

// === Inbox tests ===

#[test]
fn test_inbox_empty() {
    let (app, _temp) = create_test_app();
    let entries = app.load_and_clear_inbox("default").unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_inbox_append_and_load() {
    let (app, _temp) = create_test_app();

    let entry1 = InboxEntry {
        id: "1".to_string(),
        timestamp: 1000,
        from: "sender".to_string(),
        to: "default".to_string(),
        content: "Message 1".to_string(),
    };
    let entry2 = InboxEntry {
        id: "2".to_string(),
        timestamp: 2000,
        from: "sender".to_string(),
        to: "default".to_string(),
        content: "Message 2".to_string(),
    };

    app.append_to_inbox("default", &entry1).unwrap();
    app.append_to_inbox("default", &entry2).unwrap();

    let entries = app.load_and_clear_inbox("default").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].content, "Message 1");
    assert_eq!(entries[1].content, "Message 2");

    // Should be cleared
    let entries_after = app.load_and_clear_inbox("default").unwrap();
    assert!(entries_after.is_empty());
}

#[test]
fn test_peek_inbox_returns_entries_without_clearing() {
    let (app, _temp) = create_test_app();

    let entry = InboxEntry {
        id: "test-1".to_string(),
        timestamp: 1000,
        from: "other".to_string(),
        to: "default".to_string(),
        content: "Hello!".to_string(),
    };
    app.append_to_inbox("default", &entry).unwrap();

    // Peek should return the message
    let peeked = app.peek_inbox("default").unwrap();
    assert_eq!(peeked.len(), 1);
    assert_eq!(peeked[0].content, "Hello!");

    // Peek again - should still be there (not cleared)
    let peeked2 = app.peek_inbox("default").unwrap();
    assert_eq!(peeked2.len(), 1);

    // load_and_clear should still work
    let cleared = app.load_and_clear_inbox("default").unwrap();
    assert_eq!(cleared.len(), 1);

    // Now peek should return empty
    let peeked3 = app.peek_inbox("default").unwrap();
    assert!(peeked3.is_empty());
}

// === System prompt tests ===

#[test]
fn test_set_and_load_system_prompt() {
    let (app, _temp) = create_test_app();

    app.set_system_prompt_for("default", "You are a helpful assistant.")
        .unwrap();
    let loaded = app.load_system_prompt_for("default").unwrap();
    assert_eq!(loaded, "You are a helpful assistant.");
}

#[test]
fn test_system_prompt_fallback() {
    let (app, _temp) = create_test_app();

    // Write default prompt
    fs::write(app.prompts_dir.join("chibi.md"), "Default prompt").unwrap();

    // No context-specific prompt, should fall back
    let loaded = app.load_system_prompt_for("default").unwrap();
    assert_eq!(loaded, "Default prompt");
}

// === Config resolution tests ===

#[test]
fn test_resolve_config_defaults() {
    let (app, _temp) = create_test_app();
    let resolved = app.resolve_config("default", None).unwrap();

    assert_eq!(resolved.api_key, Some("test-key".to_string()));
    assert_eq!(resolved.model, "test-model");
    assert_eq!(resolved.username, "testuser");
}

#[test]
fn test_resolve_config_local_override() {
    let (app, _temp) = create_test_app();

    // Set local config
    let local = LocalConfig {
        model: Some("local-model".to_string()),
        username: Some("localuser".to_string()),
        auto_compact: Some(true),
        ..Default::default()
    };
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();
    assert_eq!(resolved.model, "local-model");
    assert_eq!(resolved.username, "localuser");
    assert!(resolved.auto_compact);
}

#[test]
fn test_resolve_config_username_override() {
    let (app, _temp) = create_test_app();

    // Set local config
    let local = LocalConfig {
        username: Some("localuser".to_string()),
        ..Default::default()
    };
    app.save_local_config("default", &local).unwrap();

    // Runtime username override should override local
    let resolved = app.resolve_config("default", Some("overrideuser")).unwrap();
    assert_eq!(resolved.username, "overrideuser");
}

#[test]
fn test_resolve_config_api_params_global_defaults() {
    let (app, _temp) = create_test_app();
    let resolved = app.resolve_config("default", None).unwrap();

    // Should have defaults from ApiParams::defaults()
    assert_eq!(resolved.api.prompt_caching, Some(true));
    assert_eq!(resolved.api.parallel_tool_calls, Some(true));
    assert_eq!(
        resolved.api.reasoning.effort,
        Some(crate::config::ReasoningEffort::Medium)
    );
}

#[test]
fn test_resolve_config_api_params_context_override() {
    let (app, _temp) = create_test_app();

    // Set local config with API overrides
    let local = LocalConfig {
        api: Some(ApiParams {
            temperature: Some(0.7),
            max_tokens: Some(2000),
            ..Default::default()
        }),
        ..Default::default()
    };
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();

    // Context-level API params should override
    assert_eq!(resolved.api.temperature, Some(0.7));
    assert_eq!(resolved.api.max_tokens, Some(2000));
    // But defaults should still be present for unset values
    assert_eq!(resolved.api.prompt_caching, Some(true));
}

#[test]
fn test_resolve_config_model_level_api_params() {
    // Create test app with models config
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("test-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false,
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    // Add model config
    app.models_config.models.insert(
        "test-model".to_string(),
        crate::config::ModelMetadata {
            context_window: Some(16000),
            supports_tool_calls: None,
            api: ApiParams {
                temperature: Some(0.5),
                reasoning: crate::config::ReasoningConfig {
                    effort: Some(crate::config::ReasoningEffort::High),
                    ..Default::default()
                },
                ..Default::default()
            },
        },
    );

    let resolved = app.resolve_config("default", None).unwrap();

    // Model-level params should be applied
    assert_eq!(resolved.api.temperature, Some(0.5));
    assert_eq!(
        resolved.api.reasoning.effort,
        Some(crate::config::ReasoningEffort::High)
    );
    // Model context window should override
    assert_eq!(resolved.context_window_limit, 16000);
}

#[test]
fn test_resolve_config_hierarchy_context_over_model() {
    // Test that context-level API params override model-level
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("test-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false,
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    // Add model config with temperature
    app.models_config.models.insert(
        "test-model".to_string(),
        crate::config::ModelMetadata {
            context_window: Some(16000),
            supports_tool_calls: None,
            api: ApiParams {
                temperature: Some(0.5),
                max_tokens: Some(1000),
                ..Default::default()
            },
        },
    );

    // Set local config that overrides temperature but not max_tokens
    let local = LocalConfig {
        api: Some(ApiParams {
            temperature: Some(0.9), // Override model's 0.5
            ..Default::default()
        }),
        ..Default::default()
    };
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();

    // Context should override model
    assert_eq!(resolved.api.temperature, Some(0.9));
    // Model value should be preserved when context doesn't override
    assert_eq!(resolved.api.max_tokens, Some(1000));
}

// NOTE: test_resolve_config_cli_persistent_username and
// test_resolve_config_cli_temp_username_over_persistent were removed in the
// stateless-core refactor. The distinction between persistent (-u) and
// ephemeral (-U) usernames is now handled by the CLI layer, not chibi-core.
// Core only knows about a single optional username_override parameter.

#[test]
fn test_resolve_config_all_local_overrides() {
    let (app, _temp) = create_test_app();

    // Set all local config overrides
    let local = LocalConfig {
        model: Some("local-model".to_string()),
        api_key: Some("local-key".to_string()),
        username: Some("localuser".to_string()),
        verbose: None,
        hide_tool_calls: None,
        no_tool_calls: None,
        auto_compact: Some(true),
        auto_compact_threshold: Some(90.0),
        fuel: Some(50),
        fuel_empty_response_cost: None,
        warn_threshold_percent: Some(85.0),
        context_window_limit: Some(16000),
        reflection_enabled: Some(false),
        reflection_character_limit: None,
        rolling_compact_drop_percentage: None,
        tool_output_cache_threshold: None,
        tool_cache_max_age_days: None,
        auto_cleanup_cache: None,
        tool_cache_preview_chars: None,
        file_tools_allowed_paths: None,
        api: None,
        tools: None,
        storage: StorageConfig::default(),
        fallback_tool: None,
    };
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();

    assert_eq!(resolved.model, "local-model");
    assert_eq!(resolved.api_key, Some("local-key".to_string()));
    assert_eq!(resolved.username, "localuser");
    assert!(resolved.auto_compact);
    assert!((resolved.auto_compact_threshold - 90.0).abs() < f32::EPSILON);
    assert_eq!(resolved.fuel, 50);
    assert!((resolved.warn_threshold_percent - 85.0).abs() < f32::EPSILON);
    assert_eq!(resolved.context_window_limit, 16000);
    assert!(!resolved.reflection_enabled);
}

#[test]
fn test_resolve_config_supports_tool_calls_false_disables_tools() {
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("no-tools-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false, // user hasn't disabled tools
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    // Model that doesn't support tool calls
    app.models_config.models.insert(
        "no-tools-model".to_string(),
        crate::config::ModelMetadata {
            context_window: Some(128000),
            supports_tool_calls: Some(false),
            api: ApiParams::default(),
        },
    );

    let resolved = app.resolve_config("default", None).unwrap();

    // Model capability constraint should force no_tool_calls = true
    assert!(resolved.no_tool_calls);
}

#[test]
fn test_resolve_config_supports_tool_calls_overrides_user_config() {
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("no-tools-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false,
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    app.models_config.models.insert(
        "no-tools-model".to_string(),
        crate::config::ModelMetadata {
            context_window: Some(128000),
            supports_tool_calls: Some(false),
            api: ApiParams::default(),
        },
    );

    // Even if local.toml explicitly sets no_tool_calls = false,
    // model capability should still win
    let local = LocalConfig {
        no_tool_calls: Some(false),
        ..Default::default()
    };
    app.save_local_config("default", &local).unwrap();

    let resolved = app.resolve_config("default", None).unwrap();
    assert!(
        resolved.no_tool_calls,
        "model capability constraint must override user config"
    );
}

#[test]
fn test_resolve_config_supports_tool_calls_none_preserves_default() {
    let temp_dir = TempDir::new().unwrap();
    let config = Config {
        api_key: Some("test-key".to_string()),
        model: Some("normal-model".to_string()),
        context_window_limit: Some(8000),
        warn_threshold_percent: 75.0,
        verbose: false,
        hide_tool_calls: false,
        no_tool_calls: false,
        auto_compact: false,
        auto_compact_threshold: 80.0,
        reflection_enabled: true,
        reflection_character_limit: 10000,
        fuel: 15,
        fuel_empty_response_cost: 15,
        username: "testuser".to_string(),
        lock_heartbeat_seconds: 30,
        rolling_compact_drop_percentage: 50.0,
        tool_output_cache_threshold: 4000,
        tool_cache_max_age_days: 7,
        auto_cleanup_cache: true,
        tool_cache_preview_chars: 500,
        file_tools_allowed_paths: vec![],
        api: ApiParams::default(),
        storage: StorageConfig::default(),
        fallback_tool: "call_user".to_string(),
        tools: ToolsConfig::default(),
    };

    let mut app = AppState::from_dir(temp_dir.path().to_path_buf(), config).unwrap();

    // Model with supports_tool_calls omitted (None) — should not affect no_tool_calls
    app.models_config.models.insert(
        "normal-model".to_string(),
        crate::config::ModelMetadata {
            context_window: Some(128000),
            supports_tool_calls: None,
            api: ApiParams::default(),
        },
    );

    let resolved = app.resolve_config("default", None).unwrap();
    assert!(
        !resolved.no_tool_calls,
        "None should not disable tool calls"
    );
}

// Note: Image config tests removed - image presentation is handled by CLI layer

// === Transcript entry creation tests ===

#[test]
fn test_create_user_message_entry() {
    let (_app, _temp) = create_test_app();
    let entry = create_user_message_entry("default", "Hello", "alice");

    assert!(!entry.id.is_empty());
    assert!(entry.timestamp > 0);
    assert_eq!(entry.from, "alice");
    assert_eq!(entry.to, "default");
    assert_eq!(entry.content, "Hello");
    assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_MESSAGE);
}

#[test]
fn test_create_assistant_message_entry() {
    let (_app, _temp) = create_test_app();
    let entry = create_assistant_message_entry("default", "Hi there!");

    assert_eq!(entry.from, "default");
    assert_eq!(entry.to, "user");
    assert_eq!(entry.content, "Hi there!");
    assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_MESSAGE);
}

#[test]
fn test_create_tool_call_entry() {
    let (_app, _temp) = create_test_app();
    let entry = create_tool_call_entry("default", "web_search", r#"{"query": "rust"}"#, "tc_1");

    assert_eq!(entry.from, "default");
    assert_eq!(entry.to, "web_search");
    assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_TOOL_CALL);
    assert_eq!(entry.tool_call_id, Some("tc_1".to_string()));
}

#[test]
fn test_create_tool_result_entry() {
    let (_app, _temp) = create_test_app();
    let entry = create_tool_result_entry("default", "web_search", "Search results...", "tc_1");

    assert_eq!(entry.from, "web_search");
    assert_eq!(entry.to, "default");
    assert_eq!(entry.entry_type, crate::context::ENTRY_TYPE_TOOL_RESULT);
    assert_eq!(entry.tool_call_id, Some("tc_1".to_string()));
}

// === JSONL parsing robustness tests ===

#[test]
fn test_jsonl_empty_file() {
    let (app, _temp) = create_test_app();

    // Create empty context.jsonl
    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();
    fs::write(ctx_dir.join("context.jsonl"), "").unwrap();

    // Should return empty vec, not error
    let entries = app.read_context_entries("test-context").unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_jsonl_blank_lines() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL with blank lines
    let content = r#"
{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}

{"id":"2","timestamp":1234567891,"from":"ctx","to":"user","content":"hi","entry_type":"message"}

"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].content, "hello");
    assert_eq!(entries[1].content, "hi");
}

#[test]
fn test_jsonl_malformed_entries_skipped() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL with some malformed entries
    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}
not valid json at all
{"id":"2","timestamp":1234567891,"from":"ctx","to":"user","content":"hi","entry_type":"message"}
{"incomplete": true
{"id":"3","timestamp":1234567892,"from":"user","to":"ctx","content":"bye","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    // Should skip malformed entries and return valid ones
    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 3, "Should have 3 valid entries");
    assert_eq!(entries[0].content, "hello");
    assert_eq!(entries[1].content, "hi");
    assert_eq!(entries[2].content, "bye");
}

#[test]
fn test_jsonl_nonexistent_file() {
    let (app, _temp) = create_test_app();

    // Don't create the context directory
    let entries = app.read_context_entries("nonexistent-context").unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_jsonl_unicode_content() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL with unicode content
    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"こんにちは 🎉 Привет","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "こんにちは 🎉 Привет");
}

#[test]
fn test_jsonl_with_escaped_content() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL with escaped characters in content
    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"line1\nline2\ttab","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains('\n'));
    assert!(entries[0].content.contains('\t'));
}

#[test]
fn test_jsonl_missing_optional_fields() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL without optional metadata field
    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].metadata.is_none());
}

#[test]
fn test_jsonl_with_metadata() {
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    // Write JSONL with metadata field
    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message","metadata":{"summary":"test summary"}}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries = app.read_context_entries("test-context").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].metadata.is_some());
    assert_eq!(
        entries[0].metadata.as_ref().unwrap().summary,
        Some("test summary".to_string())
    );
}

#[test]
fn test_jsonl_transcript_vs_context_entries() {
    // read_jsonl_transcript and read_context_entries should behave the same
    let (app, _temp) = create_test_app();

    let ctx_dir = app.context_dir("test-context");
    fs::create_dir_all(&ctx_dir).unwrap();

    let content = r#"{"id":"1","timestamp":1234567890,"from":"user","to":"ctx","content":"hello","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let entries1 = app.read_context_entries("test-context").unwrap();
    let entries2 = app.read_jsonl_transcript("test-context").unwrap();

    assert_eq!(entries1.len(), entries2.len());
    assert_eq!(entries1[0].id, entries2[0].id);
}

// === State/directory sync tests (Issue #13) ===

#[test]
fn test_list_contexts_excludes_manually_deleted_directories() {
    // BUG: When a context directory is manually deleted (rm -r), the context
    // should not appear in list_contexts(). Currently it lingers in state.json.
    let (mut app, _temp) = create_test_app();

    // Create two contexts
    let ctx1 = Context::new("context-one");
    let ctx2 = Context::new("context-two");
    app.save_context(&ctx1).unwrap();
    app.save_context(&ctx2).unwrap();

    // Add them to state.json
    app.state.contexts.push(ContextEntry::with_created_at(
        "context-one",
        now_timestamp(),
    ));
    app.state.contexts.push(ContextEntry::with_created_at(
        "context-two",
        now_timestamp(),
    ));
    app.save().unwrap();

    // Manually delete one context's directory (simulating rm -r)
    fs::remove_dir_all(app.context_dir("context-one")).unwrap();

    // Sync state with filesystem (this happens on startup in real usage)
    app.sync_state_with_filesystem().unwrap();

    // list_contexts should NOT include the deleted context
    let contexts = app.list_contexts();
    assert!(
        !contexts.contains(&"context-one".to_string()),
        "Deleted context should not appear in list_contexts()"
    );
    assert!(contexts.contains(&"context-two".to_string()));
}

#[test]
fn test_list_contexts_only_includes_directories_not_files() {
    // BUG: Files in ~/.chibi/contexts/ should not appear as contexts
    let (mut app, _temp) = create_test_app();

    // Create a real context
    let ctx = Context::new("real-context");
    app.save_context(&ctx).unwrap();

    // Create a stray file in the contexts directory (not a context)
    let stray_file = app.contexts_dir.join("not-a-context.txt");
    fs::write(&stray_file, "stray file content").unwrap();

    // Sync state with filesystem (discovers new directories, ignores files)
    app.sync_state_with_filesystem().unwrap();

    let contexts = app.list_contexts();

    // Should include the real context
    assert!(contexts.contains(&"real-context".to_string()));

    // Should NOT include the stray file
    assert!(
        !contexts.contains(&"not-a-context.txt".to_string()),
        "Stray files should not appear as contexts"
    );
}

#[test]
fn test_save_and_register_context_adds_to_state_contexts() {
    // Verify that save_and_register_context adds new contexts to state.json
    let (app, _temp) = create_test_app();

    // First save initial state so state.json exists
    app.save().unwrap();

    assert!(!app.state.contexts.iter().any(|e| e.name == "new-context"));

    let ctx = Context::new("new-context");
    app.save_and_register_context(&ctx).unwrap();

    // Check the state file directly
    let state_content = fs::read_to_string(&app.state_path).unwrap();
    assert!(
        state_content.contains("new-context"),
        "New context should be added to state.json"
    );
}

// NOTE: test_save_and_register_context_preserves_disk_current_context was removed
// in the stateless-core refactor. current_context is no longer stored in state.json;
// it's now managed by the CLI Session layer.

// === Touch context tests ===

#[test]
fn test_touch_context_with_destroy_settings_on_new_context() {
    let (mut app, _temp) = create_test_app();

    // Simulate what happens when switching to a new context with debug settings:
    // 1. Context entry is added to state.contexts (our fix)
    app.state.contexts.push(ContextEntry::with_created_at(
        "new-test-context",
        now_timestamp(),
    ));

    // 2. Debug settings are applied via touch_context_with_destroy_settings
    let result = app
        .touch_context_with_destroy_settings("new-test-context", None, Some(60))
        .unwrap();
    assert!(
        result,
        "Should successfully apply debug settings to new context"
    );

    // 3. Verify the destroy settings were actually saved
    let entry = app
        .state
        .contexts
        .iter()
        .find(|e| e.name == "new-test-context")
        .unwrap();
    assert_eq!(entry.destroy_after_seconds_inactive, 60);
    assert_eq!(entry.destroy_at, 0);
    assert!(
        entry.last_activity_at > 0,
        "last_activity_at should be updated by touch"
    );
}

// === Auto-destroy tests ===

#[test]
fn test_auto_destroy_expired_contexts_by_timestamp() {
    let (mut app, _temp) = create_test_app();

    // Create a context to be destroyed
    let ctx = Context::new("to-destroy");
    app.save_context(&ctx).unwrap();

    // Add entry to state.contexts with destroy_at in the past
    let mut entry = ContextEntry::with_created_at("to-destroy", now_timestamp());
    entry.destroy_at = 1; // Way in the past
    app.state.contexts.push(entry);

    // Run auto-destroy
    let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
    assert_eq!(destroyed, vec!["to-destroy".to_string()]);
    assert!(!app.context_dir("to-destroy").exists());
}

#[test]
fn test_auto_destroy_expired_contexts_by_inactivity() {
    let (mut app, _temp) = create_test_app();

    // Create a context to be destroyed
    let ctx = Context::new("to-destroy");
    app.save_context(&ctx).unwrap();

    // Add entry to state.contexts with inactivity timeout triggered
    let mut entry = ContextEntry::with_created_at("to-destroy", now_timestamp());
    entry.last_activity_at = 1; // Way in the past
    entry.destroy_after_seconds_inactive = 60; // 1 minute
    app.state.contexts.push(entry);

    // Run auto-destroy
    let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
    assert_eq!(destroyed, vec!["to-destroy".to_string()]);
    assert!(!app.context_dir("to-destroy").exists());
}

// NOTE: test_auto_destroy_skips_current_context was removed in the stateless-core
// refactor. Core no longer tracks "current context" - that's CLI's responsibility.
// auto_destroy_expired_contexts now destroys all expired contexts unconditionally.

// NOTE: test_auto_destroy_clears_previous_context_reference was removed in the
// stateless-core refactor. previous_context is now CLI session state.

#[test]
fn test_auto_destroy_respects_disabled_settings() {
    let (mut app, _temp) = create_test_app();

    // Create a context
    let ctx = Context::new("keep-context");
    app.save_context(&ctx).unwrap();

    // Add entry to state.contexts with settings that should NOT trigger destroy
    let mut entry = ContextEntry::with_created_at("keep-context", now_timestamp());
    entry.last_activity_at = 1; // Way in the past
    entry.destroy_after_seconds_inactive = 0; // Disabled
    entry.destroy_at = 0; // Disabled
    app.state.contexts.push(entry);

    // Run auto-destroy - should NOT destroy since both are disabled
    let destroyed = app.auto_destroy_expired_contexts(false).unwrap();
    assert!(destroyed.is_empty());
    assert!(app.context_dir("keep-context").exists());
}

// === Active state caching tests (Issue #1) ===

#[test]
fn test_append_to_transcript_caches_state() {
    let (app, _temp) = create_test_app();

    // Create context (save_context writes a context_created anchor, populating the cache)
    let ctx = Context::new("test-context");
    app.save_context(&ctx).unwrap();
    let count_after_save = app
        .active_state_cache
        .borrow()
        .get("test-context")
        .map(|s| s.entry_count())
        .unwrap_or(0);

    // Append an explicit entry
    let entry = create_user_message_entry("test-context", "Hello", "testuser");
    app.append_to_transcript("test-context", &entry).unwrap();

    // Cache should exist and have incremented
    let cache = app.active_state_cache.borrow();
    assert!(
        cache.contains_key("test-context"),
        "cache should contain entry after append"
    );
    assert_eq!(
        cache.get("test-context").unwrap().entry_count(),
        count_after_save + 1,
        "cache entry_count should increment by 1 after append"
    );
}

#[test]
fn test_append_to_transcript_updates_cache_incrementally() {
    let (app, _temp) = create_test_app();

    // Create context
    let ctx = Context::new("test-context");
    app.save_context(&ctx).unwrap();

    let entry1 = create_user_message_entry("test-context", "Hello", "testuser");
    app.append_to_transcript("test-context", &entry1).unwrap();
    let count_after_first = app
        .active_state_cache
        .borrow()
        .get("test-context")
        .unwrap()
        .entry_count();

    let entry2 = create_user_message_entry("test-context", "World", "testuser");
    app.append_to_transcript("test-context", &entry2).unwrap();

    // Cache should have incremented by exactly 1 from the second append
    let cache = app.active_state_cache.borrow();
    assert_eq!(
        cache.get("test-context").unwrap().entry_count(),
        count_after_first + 1,
        "cache should increment by 1 after each append"
    );
}

#[test]
fn test_destroy_context_invalidates_cache() {
    let (mut app, _temp) = create_test_app();

    // Create context and populate cache
    let ctx = Context::new("test-context");
    app.save_context(&ctx).unwrap();

    let entry = create_user_message_entry("test-context", "Hello", "testuser");
    app.append_to_transcript("test-context", &entry).unwrap();

    // Verify cache has entry
    assert!(app.active_state_cache.borrow().contains_key("test-context"));

    // Destroy context
    app.destroy_context("test-context").unwrap();

    // Cache should be invalidated
    assert!(
        !app.active_state_cache.borrow().contains_key("test-context"),
        "cache should be invalidated after destroy_context"
    );
}

#[test]
fn test_finalize_compaction_invalidates_cache() {
    let (app, _temp) = create_test_app();

    // Create context and populate cache
    let ctx = Context::new("test-context");
    app.save_context(&ctx).unwrap();

    let entry = create_user_message_entry("test-context", "Hello", "testuser");
    app.append_to_transcript("test-context", &entry).unwrap();

    // Verify cache has entry
    assert!(app.active_state_cache.borrow().contains_key("test-context"));

    // Finalize compaction (writes another entry via append_to_transcript,
    // then invalidates cache)
    app.finalize_compaction("test-context", "Test summary")
        .unwrap();

    // Cache should be invalidated
    assert!(
        !app.active_state_cache.borrow().contains_key("test-context"),
        "cache should be invalidated after finalize_compaction"
    );
}

#[test]
fn test_clear_context_invalidates_cache() {
    let (app, _temp) = create_test_app();

    // Create the "default" context with messages so clear_context has something to clear
    let mut ctx = Context::new("default");
    ctx.messages.push(json!({
        "_id": uuid::Uuid::new_v4().to_string(),
        "role": "user",
        "content": "Hello"
    }));
    app.save_context(&ctx).unwrap();

    // Populate cache explicitly
    let entry = create_user_message_entry("test-context", "Hello", "testuser");
    app.append_to_transcript("default", &entry).unwrap();
    assert!(app.active_state_cache.borrow().contains_key("default"));

    // clear_context writes archival anchor and saves fresh context (both populate
    // cache), then invalidates the cache as the final step
    app.clear_context("default").unwrap();

    // Cache should be absent after clear
    assert!(
        !app.active_state_cache.borrow().contains_key("default"),
        "cache should be invalidated after clear_context"
    );
}

// === Tool history preservation tests ===

#[test]
fn test_entries_to_messages_includes_tool_calls() {
    let (app, _temp) = create_test_app();

    let entries = vec![
        create_user_message_entry("ctx", "do something", "testuser"),
        create_tool_call_entry("ctx", "web_search", r#"{"query":"rust"}"#, "tc_1"),
        create_tool_result_entry("ctx", "web_search", "search results here", "tc_1"),
        create_assistant_message_entry("ctx", "here are the results"),
    ];

    let messages = app.entries_to_messages(&entries);

    assert_eq!(messages.len(), 4); // user, assistant+tool_calls, tool_result, assistant
    assert_eq!(messages[0]["role"].as_str().unwrap(), "user");
    assert_eq!(messages[1]["role"].as_str().unwrap(), "assistant");
    assert!(messages[1]["tool_calls"].is_array());
    assert_eq!(messages[2]["role"].as_str().unwrap(), "tool");
    assert_eq!(messages[2]["tool_call_id"].as_str().unwrap(), "tc_1");
    assert_eq!(messages[3]["role"].as_str().unwrap(), "assistant");
    assert_eq!(
        messages[3]["content"].as_str().unwrap(),
        "here are the results"
    );
}

#[test]
fn test_entries_to_messages_groups_tool_batch() {
    let (app, _temp) = create_test_app();

    // Simulate a batch of two tool calls (written in order: tc, tc, tr, tr)
    let entries = vec![
        create_tool_call_entry("ctx", "tool_a", r#"{"a":1}"#, "tc_a"),
        create_tool_call_entry("ctx", "tool_b", r#"{"b":2}"#, "tc_b"),
        create_tool_result_entry("ctx", "tool_a", "result_a", "tc_a"),
        create_tool_result_entry("ctx", "tool_b", "result_b", "tc_b"),
    ];

    let messages = app.entries_to_messages(&entries);

    // Should produce: 1 assistant message with 2 tool_calls, then 2 tool result messages
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"].as_str().unwrap(), "assistant");
    let tool_calls = messages[0]["tool_calls"].as_array().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(
        tool_calls[0]["function"]["name"].as_str().unwrap(),
        "tool_a"
    );
    assert_eq!(
        tool_calls[1]["function"]["name"].as_str().unwrap(),
        "tool_b"
    );
    assert_eq!(messages[1]["role"].as_str().unwrap(), "tool");
    assert_eq!(messages[1]["tool_call_id"].as_str().unwrap(), "tc_a");
    assert_eq!(messages[2]["role"].as_str().unwrap(), "tool");
    assert_eq!(messages[2]["tool_call_id"].as_str().unwrap(), "tc_b");
}

#[test]
fn test_entries_to_messages_backward_compat_no_tool_call_id() {
    let (app, _temp) = create_test_app();

    // Old-style entries without tool_call_id (pre-migration)
    let tc_entry = TranscriptEntry::builder()
        .from("ctx")
        .to("web_search")
        .content(r#"{"query":"test"}"#)
        .entry_type(crate::context::ENTRY_TYPE_TOOL_CALL)
        .build();
    let tr_entry = TranscriptEntry::builder()
        .from("web_search")
        .to("ctx")
        .content("results")
        .entry_type(crate::context::ENTRY_TYPE_TOOL_RESULT)
        .build();

    let entries = vec![tc_entry.clone(), tr_entry];
    let messages = app.entries_to_messages(&entries);

    assert_eq!(messages.len(), 2);
    // Assistant message with synthetic tool_call_id
    let tc_json = &messages[0]["tool_calls"].as_array().unwrap()[0];
    let synthetic_id = tc_json["id"].as_str().unwrap();
    assert!(synthetic_id.starts_with("synth_"));
    // Tool result should use the same synthetic ID
    assert_eq!(messages[1]["tool_call_id"].as_str().unwrap(), synthetic_id);
}

#[test]
fn test_json_messages_to_entries_round_trip() {
    let (app, _temp) = create_test_app();

    let original_messages = vec![
        json!({"_id": "m1", "role": "user", "content": "hello"}),
        json!({
            "_id": "m2",
            "role": "assistant",
            "tool_calls": [{
                "id": "tc_1",
                "type": "function",
                "function": {"name": "search", "arguments": r#"{"q":"test"}"#}
            }]
        }),
        json!({"_id": "m3", "role": "tool", "tool_call_id": "tc_1", "content": "found it"}),
        json!({"_id": "m4", "role": "assistant", "content": "here you go"}),
    ];

    // Messages → entries → messages should preserve structure
    let entries = app.messages_to_entries(&original_messages, "ctx");
    let roundtripped = app.entries_to_messages(&entries);

    assert_eq!(roundtripped.len(), 4);
    assert_eq!(roundtripped[0]["role"].as_str().unwrap(), "user");
    assert_eq!(roundtripped[0]["content"].as_str().unwrap(), "hello");
    assert_eq!(roundtripped[1]["role"].as_str().unwrap(), "assistant");
    assert!(roundtripped[1]["tool_calls"].is_array());
    assert_eq!(roundtripped[2]["role"].as_str().unwrap(), "tool");
    assert_eq!(roundtripped[2]["tool_call_id"].as_str().unwrap(), "tc_1");
    assert_eq!(roundtripped[3]["role"].as_str().unwrap(), "assistant");
    assert_eq!(roundtripped[3]["content"].as_str().unwrap(), "here you go");
}

#[test]
fn test_save_load_preserves_tool_history() {
    let (app, _temp) = create_test_app();

    let context = Context {
        name: "tool-test".to_string(),
        messages: vec![
            json!({"_id": "m1", "role": "user", "content": "search for rust"}),
            json!({
                "_id": "m2",
                "role": "assistant",
                "tool_calls": [{
                    "id": "tc_1",
                    "type": "function",
                    "function": {"name": "web_search", "arguments": r#"{"query":"rust"}"#}
                }]
            }),
            json!({"_id": "m3", "role": "tool", "tool_call_id": "tc_1", "content": "Rust is a programming language"}),
            json!({"_id": "m4", "role": "assistant", "content": "Rust is a systems programming language."}),
        ],
        created_at: 1234567890,
        updated_at: 1234567891,
        summary: String::new(),
    };

    app.save_context(&context).unwrap();
    let loaded = app.load_context("tool-test").unwrap();

    // Tool history should be preserved across save/load
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[0]["role"].as_str().unwrap(), "user");
    assert_eq!(loaded.messages[1]["role"].as_str().unwrap(), "assistant");
    assert!(loaded.messages[1]["tool_calls"].is_array());
    let tool_calls = loaded.messages[1]["tool_calls"].as_array().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0]["function"]["name"].as_str().unwrap(),
        "web_search"
    );
    assert_eq!(loaded.messages[2]["role"].as_str().unwrap(), "tool");
    assert_eq!(loaded.messages[2]["tool_call_id"].as_str().unwrap(), "tc_1");
    assert_eq!(loaded.messages[3]["role"].as_str().unwrap(), "assistant");
}

#[test]
fn test_backward_compat_old_context_jsonl_without_tool_call_id() {
    let (app, _temp) = create_test_app();

    // Simulate old-format context.jsonl (entries without tool_call_id field)
    let ctx_dir = app.context_dir("old-format");
    fs::create_dir_all(&ctx_dir).unwrap();

    let content = r#"{"id":"1","timestamp":1000,"from":"user","to":"ctx","content":"hello","entry_type":"message"}
{"id":"2","timestamp":1001,"from":"ctx","to":"web_search","content":"{\"q\":\"test\"}","entry_type":"tool_call"}
{"id":"3","timestamp":1002,"from":"web_search","to":"ctx","content":"results","entry_type":"tool_result"}
{"id":"4","timestamp":1003,"from":"ctx","to":"user","content":"done","entry_type":"message"}"#;
    fs::write(ctx_dir.join("context.jsonl"), content).unwrap();

    let loaded = app.load_context("old-format").unwrap();

    // Should load all entries including tool history
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[0]["role"].as_str().unwrap(), "user");
    assert_eq!(loaded.messages[1]["role"].as_str().unwrap(), "assistant");
    assert!(loaded.messages[1]["tool_calls"].is_array());
    assert_eq!(loaded.messages[2]["role"].as_str().unwrap(), "tool");
    assert_eq!(loaded.messages[3]["role"].as_str().unwrap(), "assistant");
}
