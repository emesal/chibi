use crate::config::{Config, LocalConfig, ModelsConfig, ResolvedConfig};
use crate::context::{Context, ContextState, Message, TranscriptEntry, validate_context_name, now_timestamp};
use dirs_next::home_dir;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub models_config: ModelsConfig,
    pub state: ContextState,
    pub state_path: PathBuf,
    pub chibi_dir: PathBuf,
    pub contexts_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub tools_dir: PathBuf,
}

impl AppState {
    pub fn load() -> io::Result<Self> {
        let home = home_dir().ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Home directory not found"))?;
        let chibi_dir = home.join(".chibi");
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        let tools_dir = chibi_dir.join("tools");

        // Create directories if they don't exist
        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        fs::create_dir_all(&tools_dir)?;

        let config_path = chibi_dir.join("config.toml");
        let models_path = chibi_dir.join("models.toml");
        let state_path = chibi_dir.join("state.json");

        let config: Config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            toml::from_str(&content)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse config: {}", e)))?
        } else {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Config file not found at {}. Please create config.toml with api_key, model, context_window_limit, and warn_threshold_percent", config_path.display()),
            ));
        };

        // Load models.toml (optional)
        let models_config: ModelsConfig = if models_path.exists() {
            let content = fs::read_to_string(&models_path)?;
            toml::from_str(&content)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse models.toml: {}", e)))?
        } else {
            ModelsConfig::default()
        };

        let state = if state_path.exists() {
            let file = File::open(&state_path)?;
            serde_json::from_reader(BufReader::new(file))
                .unwrap_or_else(|e| {
                    eprintln!("[WARN] State file corrupted, resetting to defaults: {}", e);
                    ContextState {
                        contexts: Vec::new(),
                        current_context: "default".to_string(),
                    }
                })
        } else {
            ContextState {
                contexts: Vec::new(),
                current_context: "default".to_string(),
            }
        };

        Ok(AppState {
            config,
            models_config,
            state,
            state_path,
            chibi_dir,
            contexts_dir,
            prompts_dir,
            tools_dir,
        })
    }
    
    pub fn save(&self) -> io::Result<()> {
        self.state.save(&self.state_path)
    }
    
    pub fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir.join(name)
    }
    
    pub fn context_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.json")
    }
    
    pub fn transcript_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.txt")
    }

    pub fn summary_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("summary.md")
    }
    
    pub fn ensure_context_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.context_dir(name);
        fs::create_dir_all(&dir)
    }
    
    pub fn load_context(&self, name: &str) -> io::Result<Context> {
        let file = File::open(self.context_file(name))?;
        let mut context: Context = serde_json::from_reader(BufReader::new(file))
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse context '{}': {}", name, e)))?;

        // Load summary from separate file
        let summary_path = self.summary_file(name);
        if summary_path.exists() {
            context.summary = fs::read_to_string(&summary_path)?;
        }

        Ok(context)
    }
    
    pub fn save_context(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.context_file(&context.name))?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, context)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to save context: {}", e)))?;

        // Save summary to separate file
        let summary_path = self.summary_file(&context.name);
        if !context.summary.is_empty() {
            fs::write(&summary_path, &context.summary)?;
        } else if summary_path.exists() {
            // Remove empty summary file
            fs::remove_file(&summary_path)?;
        }

        Ok(())
    }
    
    pub fn append_to_transcript(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(self.transcript_file(&context.name))?;
        
        for msg in &context.messages {
            // Skip system messages to avoid cluttering transcript with boilerplate
            if msg.role == "system" {
                continue;
            }
            writeln!(file, "[{}]: {}\n", msg.role.to_uppercase(), msg.content)?;
        }
        
        Ok(())
    }
    
    pub fn get_current_context(&self) -> io::Result<Context> {
        self.load_context(&self.state.current_context)
            .or_else(|e| {
                if e.kind() == ErrorKind::NotFound {
                    // Return empty context if it doesn't exist yet
                    Ok(Context {
                        name: self.state.current_context.clone(),
                        messages: Vec::new(),
                        created_at: now_timestamp(),
                        updated_at: 0,
                        summary: String::new(),
                    })
                } else {
                    Err(e)
                }
            })
    }
    
    pub fn save_current_context(&self, context: &Context) -> io::Result<()> {
        self.save_context(context)?;
        
        // Ensure the context is tracked in state
        if !self.state.contexts.contains(&context.name) {
            let mut new_state = self.state.clone();
            new_state.contexts.push(context.name.clone());
            new_state.save(&self.state_path)?;
        }
        
        Ok(())
    }
    
    pub fn add_message(&self, context: &mut Context, role: String, content: String) {
        context.messages.push(Message { role, content });
        context.updated_at = now_timestamp();
    }
    
    pub fn clear_context(&self) -> io::Result<()> {
        let context = self.get_current_context()?;
        
        // Don't clear if already empty
        if context.messages.is_empty() {
            return Ok(());
        }
        
        // Append to transcript before clearing
        self.append_to_transcript(&context)?;
        
        // Create fresh context (preserving nothing - full clear)
        let new_context = Context {
            name: self.state.current_context.clone(),
            messages: Vec::new(),
            created_at: now_timestamp(),
            updated_at: 0,
            summary: String::new(),
        };

        self.save_current_context(&new_context)?;
        Ok(())
    }

    pub fn delete_context(&self, name: &str) -> io::Result<bool> {
        if self.state.current_context == name {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("Cannot delete the current context '{}'. Switch to another context first.", name),
            ));
        }
        
        let dir = self.context_dir(name);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
            // Remove from state
            let mut new_state = self.state.clone();
            new_state.contexts.retain(|c| c != name);
            new_state.save(&self.state_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    pub fn rename_context(&self, old_name: &str, new_name: &str) -> io::Result<()> {
        validate_context_name(new_name)?;
        
        let old_dir = self.context_dir(old_name);
        let new_dir = self.context_dir(new_name);
        
        if !old_dir.exists() {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Context '{}' does not exist", old_name),
            ));
        }
        
        if new_dir.exists() {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("Context '{}' already exists", new_name),
            ));
        }
        
        // Rename the directory
        fs::rename(&old_dir, &new_dir)?;
        
        // Update context file name if needed
        let new_context_file = self.context_file(new_name);
        if new_context_file.exists() {
            let file = File::open(&new_context_file)?;
            let mut context: Context = serde_json::from_reader(BufReader::new(file))
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse context: {}", e)))?;
            
            context.name = new_name.to_string();
            
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&new_context_file)?;
            serde_json::to_writer_pretty(BufWriter::new(file), &context)
                .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to save context: {}", e)))?;
        }
        
        // Update state
        let mut new_state = self.state.clone();
        if new_state.current_context == old_name {
            new_state.current_context = new_name.to_string();
        }
        new_state.contexts.retain(|c| c != old_name);
        if !new_state.contexts.contains(&new_name.to_string()) {
            new_state.contexts.push(new_name.to_string());
        }
        new_state.save(&self.state_path)?;
        
        Ok(())
    }
    
    pub fn list_contexts(&self) -> Vec<String> {
        // Scan contexts directory
        let mut contexts = self.state.contexts.clone();
        
        if let Ok(entries) = fs::read_dir(&self.contexts_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !contexts.contains(&name) {
                    contexts.push(name);
                }
            }
        }
        
        contexts.sort();
        contexts
    }
    
    pub fn calculate_token_count(&self, messages: &[Message]) -> usize {
        // Rough estimation: 4 chars per token on average
        messages.iter().map(|m| (m.content.len() + m.role.len()) / 4).sum()
    }
    
    pub fn should_warn(&self, messages: &[Message]) -> bool {
        let tokens = self.calculate_token_count(messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.warn_threshold_percent
    }
    
    pub fn remaining_tokens(&self, messages: &[Message]) -> usize {
        let tokens = self.calculate_token_count(messages);
        if tokens >= self.config.context_window_limit {
            0
        } else {
            self.config.context_window_limit - tokens
        }
    }
    
    pub fn load_prompt(&self, name: &str) -> io::Result<String> {
        let prompt_path = self.prompts_dir.join(format!("{}.md", name));
        if prompt_path.exists() {
            fs::read_to_string(&prompt_path)
        } else {
            Ok(String::new())
        }
    }

    /// Get the path to a context's system prompt file
    pub fn context_prompt_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("system_prompt.md")
    }

    /// Load the system prompt for the current context.
    /// Returns context-specific prompt if it exists, otherwise falls back to default.
    pub fn load_system_prompt(&self) -> io::Result<String> {
        let context_prompt_path = self.context_prompt_file(&self.state.current_context);
        if context_prompt_path.exists() {
            fs::read_to_string(&context_prompt_path)
        } else {
            // Fall back to default prompt
            self.load_prompt("chibi")
        }
    }

    /// Set a custom system prompt for the current context.
    pub fn set_system_prompt(&self, content: &str) -> io::Result<()> {
        self.ensure_context_dir(&self.state.current_context)?;
        let prompt_path = self.context_prompt_file(&self.state.current_context);
        fs::write(&prompt_path, content)
    }

    pub fn should_auto_compact(&self, context: &Context) -> bool {
        if !self.config.auto_compact {
            return false;
        }
        let tokens = self.calculate_token_count(&context.messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.auto_compact_threshold
    }

    /// Load the reflection prompt from ~/.chibi/prompts/reflection.md
    /// Returns empty string if the file doesn't exist
    pub fn load_reflection_prompt(&self) -> io::Result<String> {
        let reflection_path = self.prompts_dir.join("reflection.md");
        if reflection_path.exists() {
            fs::read_to_string(&reflection_path)
        } else {
            Ok(String::new())
        }
    }

    // --- Todos and Goals file helpers ---

    /// Get the path to a context's todos file
    pub fn todos_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("todos.md")
    }

    /// Get the path to a context's goals file
    pub fn goals_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("goals.md")
    }

    /// Load todos for a context (returns empty string if file doesn't exist)
    pub fn load_todos(&self, context_name: &str) -> io::Result<String> {
        let path = self.todos_file(context_name);
        if path.exists() {
            fs::read_to_string(&path)
        } else {
            Ok(String::new())
        }
    }

    /// Load goals for a context (returns empty string if file doesn't exist)
    pub fn load_goals(&self, context_name: &str) -> io::Result<String> {
        let path = self.goals_file(context_name);
        if path.exists() {
            fs::read_to_string(&path)
        } else {
            Ok(String::new())
        }
    }

    /// Save todos for a context
    pub fn save_todos(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        fs::write(self.todos_file(context_name), content)
    }

    /// Save goals for a context
    pub fn save_goals(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        fs::write(self.goals_file(context_name), content)
    }

    /// Load todos for current context
    pub fn load_current_todos(&self) -> io::Result<String> {
        self.load_todos(&self.state.current_context)
    }

    /// Load goals for current context
    pub fn load_current_goals(&self) -> io::Result<String> {
        self.load_goals(&self.state.current_context)
    }

    /// Save todos for current context
    pub fn save_current_todos(&self, content: &str) -> io::Result<()> {
        self.save_todos(&self.state.current_context, content)
    }

    /// Save goals for current context
    pub fn save_current_goals(&self, content: &str) -> io::Result<()> {
        self.save_goals(&self.state.current_context, content)
    }

    /// Get the path to a context's local config file
    pub fn local_config_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("local.toml")
    }

    /// Load local config for a context (returns default if doesn't exist)
    pub fn load_local_config(&self, context_name: &str) -> io::Result<LocalConfig> {
        let path = self.local_config_file(context_name);
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            toml::from_str(&content)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse local.toml: {}", e)))
        } else {
            Ok(LocalConfig::default())
        }
    }

    /// Save local config for a context
    pub fn save_local_config(&self, context_name: &str, local_config: &LocalConfig) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let path = self.local_config_file(context_name);
        let content = toml::to_string_pretty(local_config)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to serialize local.toml: {}", e)))?;
        fs::write(&path, content)
    }

    /// Resolve model name using models.toml aliases
    /// If the model is an alias defined in models.toml, return the full model name
    /// Otherwise return the original model name
    pub fn resolve_model_name(&self, model: &str) -> String {
        if self.models_config.models.contains_key(model) {
            // The model name itself is a key in models.toml, use it as-is
            // (models.toml maps alias -> metadata, not alias -> full name)
            model.to_string()
        } else {
            model.to_string()
        }
    }

    /// Get context window limit for a model (from models.toml if available)
    pub fn get_model_context_window(&self, model: &str) -> Option<usize> {
        self.models_config.models.get(model).and_then(|m| m.context_window)
    }

    /// Resolve the full configuration, applying overrides in order:
    /// 1. CLI flags (passed as parameters)
    /// 2. Context-local config (local.toml)
    /// 3. Global config (config.toml)
    /// 4. Models.toml (for model expansion)
    /// 5. Defaults
    pub fn resolve_config(
        &self,
        cli_username: Option<&str>,
        cli_temp_username: Option<&str>,
    ) -> io::Result<ResolvedConfig> {
        let local = self.load_local_config(&self.state.current_context)?;

        // Start with global config values
        let mut resolved = ResolvedConfig {
            api_key: self.config.api_key.clone(),
            model: self.config.model.clone(),
            context_window_limit: self.config.context_window_limit,
            base_url: self.config.base_url.clone(),
            auto_compact: self.config.auto_compact,
            auto_compact_threshold: self.config.auto_compact_threshold,
            reflection_enabled: self.config.reflection_enabled,
            reflection_character_limit: self.config.reflection_character_limit,
            max_recursion_depth: self.config.max_recursion_depth,
            warn_threshold_percent: self.config.warn_threshold_percent,
            username: self.config.username.clone(),
            lock_heartbeat_seconds: self.config.lock_heartbeat_seconds,
        };

        // Apply local config overrides
        if let Some(ref api_key) = local.api_key {
            resolved.api_key = api_key.clone();
        }
        if let Some(ref model) = local.model {
            resolved.model = model.clone();
        }
        if let Some(ref base_url) = local.base_url {
            resolved.base_url = base_url.clone();
        }
        if let Some(auto_compact) = local.auto_compact {
            resolved.auto_compact = auto_compact;
        }
        if let Some(max_recursion_depth) = local.max_recursion_depth {
            resolved.max_recursion_depth = max_recursion_depth;
        }
        if let Some(ref username) = local.username {
            resolved.username = username.clone();
        }

        // Apply CLI overrides (highest priority)
        // Note: -u (persistent) should have been saved to local.toml before calling this
        // -U (temp) overrides for this invocation only
        if let Some(username) = cli_temp_username {
            resolved.username = username.to_string();
        } else if let Some(username) = cli_username {
            resolved.username = username.to_string();
        }

        // Resolve model name and potentially override context window
        resolved.model = self.resolve_model_name(&resolved.model);
        if let Some(context_window) = self.get_model_context_window(&resolved.model) {
            resolved.context_window_limit = context_window;
        }

        Ok(resolved)
    }

    /// Get the path to the JSONL transcript file
    pub fn transcript_jsonl_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.jsonl")
    }

    /// Append an entry to the JSONL transcript
    pub fn append_to_jsonl_transcript(&self, entry: &TranscriptEntry) -> io::Result<()> {
        self.ensure_context_dir(&self.state.current_context)?;
        let path = self.transcript_jsonl_file(&self.state.current_context);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(&path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to serialize transcript entry: {}", e)))?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Create a transcript entry for a user message
    pub fn create_user_message_entry(&self, content: &str, username: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: username.to_string(),
            to: self.state.current_context.clone(),
            content: content.to_string(),
            entry_type: "message".to_string(),
        }
    }

    /// Create a transcript entry for an assistant message
    pub fn create_assistant_message_entry(&self, content: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: self.state.current_context.clone(),
            to: "user".to_string(),
            content: content.to_string(),
            entry_type: "message".to_string(),
        }
    }

    /// Create a transcript entry for a tool call
    pub fn create_tool_call_entry(&self, tool_name: &str, arguments: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: self.state.current_context.clone(),
            to: tool_name.to_string(),
            content: arguments.to_string(),
            entry_type: "tool_call".to_string(),
        }
    }

    /// Create a transcript entry for a tool result
    pub fn create_tool_result_entry(&self, tool_name: &str, result: &str) -> TranscriptEntry {
        TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: tool_name.to_string(),
            to: self.state.current_context.clone(),
            content: result.to_string(),
            entry_type: "tool_result".to_string(),
        }
    }

}
