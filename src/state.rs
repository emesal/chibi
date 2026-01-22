use crate::config::{Config, LocalConfig, ModelsConfig, ResolvedConfig};
use crate::context::{
    Context, ContextMeta, ContextState, InboxEntry, Message, TranscriptEntry, now_timestamp,
    validate_context_name,
};
use dirs_next::home_dir;
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub models_config: ModelsConfig,
    pub state: ContextState,
    pub state_path: PathBuf,
    #[allow(dead_code)]
    pub chibi_dir: PathBuf,
    pub contexts_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub plugins_dir: PathBuf,
}

impl AppState {
    /// Create AppState from a custom directory (for testing)
    #[cfg(test)]
    pub fn from_dir(chibi_dir: PathBuf, config: Config) -> io::Result<Self> {
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        let plugins_dir = chibi_dir.join("plugins");
        let state_path = chibi_dir.join("state.json");

        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        fs::create_dir_all(&plugins_dir)?;

        let state = ContextState {
            contexts: Vec::new(),
            current_context: "default".to_string(),
        };

        Ok(AppState {
            config,
            models_config: ModelsConfig::default(),
            state,
            state_path,
            chibi_dir,
            contexts_dir,
            prompts_dir,
            plugins_dir,
        })
    }

    pub fn load() -> io::Result<Self> {
        let home = home_dir()
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Home directory not found"))?;
        let chibi_dir = home.join(".chibi");
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        let plugins_dir = chibi_dir.join("plugins");

        // Create directories if they don't exist
        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        fs::create_dir_all(&plugins_dir)?;

        let config_path = chibi_dir.join("config.toml");
        let models_path = chibi_dir.join("models.toml");
        let state_path = chibi_dir.join("state.json");

        let config: Config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse config: {}", e),
                )
            })?
        } else {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!(
                    "Config file not found at {}. Please create config.toml with api_key, model, context_window_limit, and warn_threshold_percent",
                    config_path.display()
                ),
            ));
        };

        // Load models.toml (optional)
        let models_config: ModelsConfig = if models_path.exists() {
            let content = fs::read_to_string(&models_path)?;
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse models.toml: {}", e),
                )
            })?
        } else {
            ModelsConfig::default()
        };

        let state = if state_path.exists() {
            let file = File::open(&state_path)?;
            serde_json::from_reader(BufReader::new(file)).unwrap_or_else(|e| {
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
            plugins_dir,
        })
    }

    pub fn save(&self) -> io::Result<()> {
        self.state.save(&self.state_path)
    }

    pub fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir.join(name)
    }

    pub fn context_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.jsonl")
    }

    /// Path to the old context.json format (for migration)
    fn context_file_old(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.json")
    }

    /// Path to context metadata file
    pub fn context_meta_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context_meta.json")
    }

    pub fn transcript_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.md")
    }

    pub fn summary_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("summary.md")
    }

    pub fn ensure_context_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.context_dir(name);
        fs::create_dir_all(&dir)
    }

    pub fn load_context(&self, name: &str) -> io::Result<Context> {
        let context_jsonl = self.context_file(name);
        let context_json_old = self.context_file_old(name);

        // Try to load from new JSONL format first
        if context_jsonl.exists() {
            return self.load_context_from_jsonl(name);
        }

        // Check if old format exists and migrate
        if context_json_old.exists() {
            return self.migrate_and_load_context(name);
        }

        // Neither exists - return not found error
        Err(io::Error::new(
            ErrorKind::NotFound,
            format!("Context '{}' not found", name),
        ))
    }

    /// Load context from the new JSONL format
    fn load_context_from_jsonl(&self, name: &str) -> io::Result<Context> {
        let entries = self.read_context_entries(name)?;
        let meta = self.load_context_meta(name)?;

        // Load summary from separate file
        let summary_path = self.summary_file(name);
        let summary = if summary_path.exists() {
            fs::read_to_string(&summary_path)?
        } else {
            String::new()
        };

        // Convert entries to messages for backwards compatibility with existing code
        let messages = self.entries_to_messages(&entries);

        // Get updated_at from the last entry timestamp, or created_at if empty
        let updated_at = entries
            .last()
            .map(|e| e.timestamp)
            .unwrap_or(meta.created_at);

        Ok(Context {
            name: name.to_string(),
            messages,
            created_at: meta.created_at,
            updated_at,
            summary,
        })
    }

    /// Migrate from old context.json format to new context.jsonl format
    fn migrate_and_load_context(&self, name: &str) -> io::Result<Context> {
        let old_path = self.context_file_old(name);
        let file = File::open(&old_path)?;
        let old_context: Context = serde_json::from_reader(BufReader::new(file)).map_err(|e| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("Failed to parse old context '{}': {}", name, e),
            )
        })?;

        // Save metadata
        let meta = ContextMeta {
            created_at: old_context.created_at,
        };
        self.save_context_meta(name, &meta)?;

        // Convert messages to entries and save to JSONL
        let entries = self.messages_to_entries(&old_context.messages, name);
        self.write_context_entries(name, &entries)?;

        // Load summary from separate file (keep it there)
        let summary_path = self.summary_file(name);
        let summary = if summary_path.exists() {
            fs::read_to_string(&summary_path)?
        } else {
            old_context.summary.clone()
        };

        // If summary was in old context but not in file, save it
        if !old_context.summary.is_empty() && !summary_path.exists() {
            fs::write(&summary_path, &old_context.summary)?;
        }

        // Remove old context.json file
        fs::remove_file(&old_path)?;

        Ok(Context {
            name: name.to_string(),
            messages: old_context.messages,
            created_at: old_context.created_at,
            updated_at: old_context.updated_at,
            summary,
        })
    }

    /// Read entries from context.jsonl
    pub fn read_context_entries(&self, name: &str) -> io::Result<Vec<TranscriptEntry>> {
        let path = self.context_file(name);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    eprintln!("[WARN] Skipping malformed context entry: {}", e);
                }
            }
        }

        Ok(entries)
    }

    /// Write entries to context.jsonl (full rewrite)
    pub fn write_context_entries(&self, name: &str, entries: &[TranscriptEntry]) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);

        for entry in entries {
            let json = serde_json::to_string(entry)
                .map_err(|e| io::Error::other(format!("Failed to serialize entry: {}", e)))?;
            writeln!(writer, "{}", json)?;
        }

        Ok(())
    }

    /// Append a single entry to context.jsonl
    pub fn append_context_entry(&self, name: &str, entry: &TranscriptEntry) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_file(name);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("Failed to serialize entry: {}", e)))?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Load context metadata
    fn load_context_meta(&self, name: &str) -> io::Result<ContextMeta> {
        let path = self.context_meta_file(name);
        if path.exists() {
            let file = File::open(&path)?;
            serde_json::from_reader(BufReader::new(file)).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse context meta: {}", e),
                )
            })
        } else {
            Ok(ContextMeta::default())
        }
    }

    /// Save context metadata
    fn save_context_meta(&self, name: &str, meta: &ContextMeta) -> io::Result<()> {
        self.ensure_context_dir(name)?;
        let path = self.context_meta_file(name);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        serde_json::to_writer_pretty(BufWriter::new(file), meta)
            .map_err(|e| io::Error::other(format!("Failed to save context meta: {}", e)))
    }

    /// Convert transcript entries to messages (for backwards compat)
    fn entries_to_messages(&self, entries: &[TranscriptEntry]) -> Vec<Message> {
        entries
            .iter()
            .filter(|e| e.entry_type == "message")
            .map(|e| {
                let role = if e.to == "user" {
                    "assistant".to_string()
                } else {
                    "user".to_string()
                };
                Message {
                    id: e.id.clone(),
                    role,
                    content: e.content.clone(),
                }
            })
            .collect()
    }

    /// Convert messages to transcript entries (for migration)
    fn messages_to_entries(
        &self,
        messages: &[Message],
        context_name: &str,
    ) -> Vec<TranscriptEntry> {
        messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| {
                let (from, to) = if m.role == "assistant" {
                    (context_name.to_string(), "user".to_string())
                } else {
                    ("user".to_string(), context_name.to_string())
                };
                TranscriptEntry {
                    id: m.id.clone(),
                    timestamp: now_timestamp(),
                    from,
                    to,
                    content: m.content.clone(),
                    entry_type: "message".to_string(),
                }
            })
            .collect()
    }

    pub fn save_context(&self, context: &Context) -> io::Result<()> {
        self.ensure_context_dir(&context.name)?;

        // Save metadata (created_at)
        let meta = ContextMeta {
            created_at: context.created_at,
        };
        self.save_context_meta(&context.name, &meta)?;

        // Convert messages to entries and write to JSONL
        let entries = self.messages_to_entries(&context.messages, &context.name);
        self.write_context_entries(&context.name, &entries)?;

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
        self.load_context(&self.state.current_context).or_else(|e| {
            if e.kind() == ErrorKind::NotFound {
                // Return empty context if it doesn't exist yet
                Ok(Context::new(self.state.current_context.clone()))
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
        context.messages.push(Message::new(role, content));
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
        let new_context = Context::new(self.state.current_context.clone());

        self.save_current_context(&new_context)?;
        Ok(())
    }

    pub fn delete_context(&self, name: &str) -> io::Result<bool> {
        if self.state.current_context == name {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Cannot delete the current context '{}'. Switch to another context first.",
                    name
                ),
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

        // Rename the directory (context name is derived from directory, no file updates needed)
        fs::rename(&old_dir, &new_dir)?;

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
        messages
            .iter()
            .map(|m| (m.content.len() + m.role.len()) / 4)
            .sum()
    }

    pub fn should_warn(&self, messages: &[Message]) -> bool {
        let tokens = self.calculate_token_count(messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.warn_threshold_percent
    }

    pub fn remaining_tokens(&self, messages: &[Message]) -> usize {
        let tokens = self.calculate_token_count(messages);
        self.config.context_window_limit.saturating_sub(tokens)
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

    /// Load the system prompt for a specific context.
    pub fn load_system_prompt_for(&self, context_name: &str) -> io::Result<String> {
        let context_prompt_path = self.context_prompt_file(context_name);
        if context_prompt_path.exists() {
            fs::read_to_string(&context_prompt_path)
        } else {
            // Fall back to default prompt
            self.load_prompt("chibi")
        }
    }

    /// Set a custom system prompt for a specific context.
    pub fn set_system_prompt_for(&self, context_name: &str, content: &str) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let prompt_path = self.context_prompt_file(context_name);
        fs::write(&prompt_path, content)
    }

    /// Load the reflection content from ~/.chibi/prompts/reflection.md
    pub fn load_reflection(&self) -> io::Result<String> {
        self.load_reflection_prompt()
    }

    /// Save reflection content to ~/.chibi/prompts/reflection.md
    pub fn save_reflection(&self, content: &str) -> io::Result<()> {
        let reflection_path = self.prompts_dir.join("reflection.md");
        fs::write(&reflection_path, content)
    }

    /// Load todos for a specific context (alias for load_todos)
    pub fn load_todos_for(&self, context_name: &str) -> io::Result<String> {
        self.load_todos(context_name)
    }

    /// Load goals for a specific context (alias for load_goals)
    pub fn load_goals_for(&self, context_name: &str) -> io::Result<String> {
        self.load_goals(context_name)
    }

    /// Clear a context by name (archive its history)
    pub fn clear_context_by_name(&self, context_name: &str) -> io::Result<()> {
        let context = self.load_context(context_name)?;

        // Don't clear if already empty
        if context.messages.is_empty() {
            return Ok(());
        }

        // Append to transcript before clearing
        self.ensure_context_dir(context_name)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.transcript_file(context_name))?;

        for msg in &context.messages {
            if msg.role == "system" {
                continue;
            }
            writeln!(file, "[{}]: {}\n", msg.role.to_uppercase(), msg.content)?;
        }

        // Create fresh context
        let new_context = Context::new(context_name);

        self.save_context(&new_context)?;
        Ok(())
    }

    /// Send a message to another context's inbox
    pub fn send_inbox_message(&self, to_context: &str, message: &str) -> io::Result<()> {
        let entry = InboxEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: now_timestamp(),
            from: self.state.current_context.clone(),
            to: to_context.to_string(),
            content: message.to_string(),
        };
        self.append_to_inbox(to_context, &entry)
    }

    pub fn should_auto_compact(&self, context: &Context, resolved_config: &ResolvedConfig) -> bool {
        if !resolved_config.auto_compact {
            return false;
        }
        let tokens = self.calculate_token_count(&context.messages);
        let usage_percent = (tokens as f32 / resolved_config.context_window_limit as f32) * 100.0;
        usage_percent >= resolved_config.auto_compact_threshold
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
            toml::from_str(&content).map_err(|e| {
                io::Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to parse local.toml: {}", e),
                )
            })
        } else {
            Ok(LocalConfig::default())
        }
    }

    /// Save local config for a context
    pub fn save_local_config(
        &self,
        context_name: &str,
        local_config: &LocalConfig,
    ) -> io::Result<()> {
        self.ensure_context_dir(context_name)?;
        let path = self.local_config_file(context_name);
        let content = toml::to_string_pretty(local_config)
            .map_err(|e| io::Error::other(format!("Failed to serialize local.toml: {}", e)))?;
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
        self.models_config
            .models
            .get(model)
            .and_then(|m| m.context_window)
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
            warn_threshold_percent: self.config.warn_threshold_percent,
            base_url: self.config.base_url.clone(),
            auto_compact: self.config.auto_compact,
            auto_compact_threshold: self.config.auto_compact_threshold,
            max_recursion_depth: self.config.max_recursion_depth,
            username: self.config.username.clone(),
            reflection_enabled: self.config.reflection_enabled,
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
        if let Some(context_window_limit) = local.context_window_limit {
            resolved.context_window_limit = context_window_limit;
        }
        if let Some(warn_threshold_percent) = local.warn_threshold_percent {
            resolved.warn_threshold_percent = warn_threshold_percent;
        }
        if let Some(auto_compact) = local.auto_compact {
            resolved.auto_compact = auto_compact;
        }
        if let Some(auto_compact_threshold) = local.auto_compact_threshold {
            resolved.auto_compact_threshold = auto_compact_threshold;
        }
        if let Some(max_recursion_depth) = local.max_recursion_depth {
            resolved.max_recursion_depth = max_recursion_depth;
        }
        if let Some(ref username) = local.username {
            resolved.username = username.clone();
        }
        if let Some(reflection_enabled) = local.reflection_enabled {
            resolved.reflection_enabled = reflection_enabled;
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

    /// Append an entry to the context (context.jsonl) - this replaces append_to_jsonl_transcript
    pub fn append_to_jsonl_transcript(&self, entry: &TranscriptEntry) -> io::Result<()> {
        self.append_context_entry(&self.state.current_context, entry)
    }

    /// Read all entries from the context file (context.jsonl) - unified with transcript
    pub fn read_jsonl_transcript(&self, context_name: &str) -> io::Result<Vec<TranscriptEntry>> {
        self.read_context_entries(context_name)
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

    /// Get the path to a context's inbox file
    pub fn inbox_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join("inbox.jsonl")
    }

    /// Get the path to a context's inbox lock file
    pub fn inbox_lock_file(&self, context_name: &str) -> PathBuf {
        self.context_dir(context_name).join(".inbox.lock")
    }

    /// Append a message to a context's inbox with exclusive locking
    pub fn append_to_inbox(&self, context_name: &str, entry: &InboxEntry) -> io::Result<()> {
        // Ensure context directory exists
        let context_dir = self.context_dir(context_name);
        fs::create_dir_all(&context_dir)?;

        let lock_path = self.inbox_lock_file(context_name);
        let inbox_path = self.inbox_file(context_name);

        // Create/open lock file and acquire exclusive lock
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock_file.lock_exclusive()?;

        // Append to inbox
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inbox_path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::other(format!("JSON serialize error: {}", e)))?;
        writeln!(file, "{}", json)?;

        // Release lock
        lock_file.unlock()?;
        Ok(())
    }

    /// Load and clear the current context's inbox atomically
    pub fn load_and_clear_current_inbox(&self) -> io::Result<Vec<InboxEntry>> {
        let context_name = &self.state.current_context;
        let lock_path = self.inbox_lock_file(context_name);
        let inbox_path = self.inbox_file(context_name);

        // If inbox doesn't exist, return empty
        if !inbox_path.exists() {
            return Ok(Vec::new());
        }

        // Create/open lock file and acquire exclusive lock
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        lock_file.lock_exclusive()?;

        // Read all entries
        let file = File::open(&inbox_path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<InboxEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => eprintln!("[Warning: Failed to parse inbox entry: {}]", e),
            }
        }

        // Clear the inbox by truncating the file
        if !entries.is_empty() {
            File::create(&inbox_path)?; // This truncates the file
        }

        // Release lock
        lock_file.unlock()?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a test AppState with a temporary directory
    fn create_test_app() -> (AppState, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = Config {
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            context_window_limit: 8000,
            warn_threshold_percent: 75.0,
            auto_compact: false,
            auto_compact_threshold: 80.0,
            base_url: "https://test.api/v1".to_string(),
            reflection_enabled: true,
            reflection_character_limit: 10000,
            max_recursion_depth: 15,
            username: "testuser".to_string(),
            lock_heartbeat_seconds: 30,
            rolling_compact_drop_percentage: 50.0,
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
    fn test_get_current_context_creates_default() {
        let (app, _temp) = create_test_app();
        let context = app.get_current_context().unwrap();
        assert_eq!(context.name, "default");
        assert!(context.messages.is_empty());
    }

    #[test]
    fn test_save_and_load_context() {
        let (app, _temp) = create_test_app();

        let context = Context {
            name: "test-context".to_string(),
            messages: vec![
                Message::new("user", "Hello"),
                Message::new("assistant", "Hi there!"),
            ],
            created_at: 1234567890,
            updated_at: 1234567891,
            summary: "Test summary".to_string(),
        };

        app.save_context(&context).unwrap();

        let loaded = app.load_context("test-context").unwrap();
        assert_eq!(loaded.name, "test-context");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "Hello");
        assert_eq!(loaded.summary, "Test summary");
    }

    #[test]
    fn test_add_message() {
        let (app, _temp) = create_test_app();
        let mut context = app.get_current_context().unwrap();

        assert!(context.messages.is_empty());

        app.add_message(&mut context, "user".to_string(), "Test message".to_string());

        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].role, "user");
        assert_eq!(context.messages[0].content, "Test message");
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
        let (app, _temp) = create_test_app();

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
            messages: vec![Message::new("user", "Hello")],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();

        // Set it as current
        app.state.current_context = "old-name".to_string();

        // Rename
        app.rename_context("old-name", "new-name").unwrap();

        // Verify
        assert!(!app.context_dir("old-name").exists());
        assert!(app.context_dir("new-name").exists());

        let loaded = app.load_context("new-name").unwrap();
        assert_eq!(loaded.name, "new-name");
        assert_eq!(loaded.messages[0].content, "Hello");
    }

    #[test]
    fn test_rename_nonexistent_context() {
        let (app, _temp) = create_test_app();
        let result = app.rename_context("nonexistent", "new-name");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_rename_to_existing_context() {
        let (app, _temp) = create_test_app();

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
    fn test_delete_context() {
        let (mut app, _temp) = create_test_app();

        // Create context to delete
        let context = Context {
            name: "to-delete".to_string(),
            messages: vec![],
            created_at: 0,
            updated_at: 0,
            summary: String::new(),
        };
        app.save_context(&context).unwrap();

        // Make sure we're not on this context
        app.state.current_context = "default".to_string();

        // Delete
        let deleted = app.delete_context("to-delete").unwrap();
        assert!(deleted);
        assert!(!app.context_dir("to-delete").exists());
    }

    #[test]
    fn test_delete_current_context_fails() {
        let (app, _temp) = create_test_app();

        // Try to delete current context
        let result = app.delete_context("default");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot delete the current context")
        );
    }

    #[test]
    fn test_delete_nonexistent_context() {
        let (app, _temp) = create_test_app();
        let deleted = app.delete_context("nonexistent").unwrap();
        assert!(!deleted);
    }

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
        let messages = vec![
            Message::new("user", "Hello world!"), // 4+12 = 16 chars / 4 = 4 tokens
        ];
        let count = app.calculate_token_count(&messages);
        assert_eq!(count, 4); // (4 + 12) / 4 = 4
    }

    #[test]
    fn test_remaining_tokens() {
        let (app, _temp) = create_test_app();
        let messages = vec![
            Message::new("user", "x".repeat(4000)), // ~1000 tokens
        ];
        let remaining = app.remaining_tokens(&messages);
        // 8000 - ~1000 = ~7000
        assert!(remaining < 8000);
        assert!(remaining > 6000);
    }

    #[test]
    fn test_should_warn() {
        let (app, _temp) = create_test_app();

        // Small message shouldn't warn
        let small_messages = vec![Message::new("user", "Hello")];
        assert!(!app.should_warn(&small_messages));

        // Large message should warn (above 75% of 8000 = 6000 tokens = ~24000 chars)
        let large_messages = vec![Message::new("user", "x".repeat(30000))];
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
        let entries = app.load_and_clear_current_inbox().unwrap();
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

        let entries = app.load_and_clear_current_inbox().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "Message 1");
        assert_eq!(entries[1].content, "Message 2");

        // Should be cleared
        let entries_after = app.load_and_clear_current_inbox().unwrap();
        assert!(entries_after.is_empty());
    }

    // === System prompt tests ===

    #[test]
    fn test_set_and_load_system_prompt() {
        let (app, _temp) = create_test_app();

        app.set_system_prompt_for(&app.state.current_context, "You are a helpful assistant.")
            .unwrap();
        let loaded = app.load_system_prompt().unwrap();
        assert_eq!(loaded, "You are a helpful assistant.");
    }

    #[test]
    fn test_system_prompt_fallback() {
        let (app, _temp) = create_test_app();

        // Write default prompt
        fs::write(app.prompts_dir.join("chibi.md"), "Default prompt").unwrap();

        // No context-specific prompt, should fall back
        let loaded = app.load_system_prompt().unwrap();
        assert_eq!(loaded, "Default prompt");
    }

    // === Config resolution tests ===

    #[test]
    fn test_resolve_config_defaults() {
        let (app, _temp) = create_test_app();
        let resolved = app.resolve_config(None, None).unwrap();

        assert_eq!(resolved.api_key, "test-key");
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

        let resolved = app.resolve_config(None, None).unwrap();
        assert_eq!(resolved.model, "local-model");
        assert_eq!(resolved.username, "localuser");
        assert!(resolved.auto_compact);
    }

    #[test]
    fn test_resolve_config_cli_override() {
        let (app, _temp) = create_test_app();

        // Set local config
        let local = LocalConfig {
            username: Some("localuser".to_string()),
            ..Default::default()
        };
        app.save_local_config("default", &local).unwrap();

        // CLI temp username should override local
        let resolved = app.resolve_config(None, Some("cliuser")).unwrap();
        assert_eq!(resolved.username, "cliuser");
    }

    // === Transcript entry creation tests ===

    #[test]
    fn test_create_user_message_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_user_message_entry("Hello", "alice");

        assert!(!entry.id.is_empty());
        assert!(entry.timestamp > 0);
        assert_eq!(entry.from, "alice");
        assert_eq!(entry.to, "default");
        assert_eq!(entry.content, "Hello");
        assert_eq!(entry.entry_type, "message");
    }

    #[test]
    fn test_create_assistant_message_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_assistant_message_entry("Hi there!");

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "user");
        assert_eq!(entry.content, "Hi there!");
        assert_eq!(entry.entry_type, "message");
    }

    #[test]
    fn test_create_tool_call_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_tool_call_entry("web_search", r#"{"query": "rust"}"#);

        assert_eq!(entry.from, "default");
        assert_eq!(entry.to, "web_search");
        assert_eq!(entry.entry_type, "tool_call");
    }

    #[test]
    fn test_create_tool_result_entry() {
        let (app, _temp) = create_test_app();
        let entry = app.create_tool_result_entry("web_search", "Search results...");

        assert_eq!(entry.from, "web_search");
        assert_eq!(entry.to, "default");
        assert_eq!(entry.entry_type, "tool_result");
    }
}
