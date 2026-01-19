use crate::config::Config;
use crate::context::{Context, ContextState, Message, validate_context_name, now_timestamp};
use dirs_next::home_dir;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;

#[derive(Debug)]
pub struct AppState {
    pub config: Config,
    pub state: ContextState,
    pub state_path: PathBuf,
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
        let state_path = chibi_dir.join("state.json");
        
        let config = if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            toml::from_str(&content)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse config: {}", e)))?
        } else {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                format!("Config file not found at {}. Please create config.toml with api_key, model, context_window_limit, and warn_threshold_percent", config_path.display()),
            ));
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
            state,
            state_path,
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
    
    pub fn ensure_context_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.context_dir(name);
        fs::create_dir_all(&dir)
    }
    
    pub fn load_context(&self, name: &str) -> io::Result<Context> {
        let file = File::open(self.context_file(name))?;
        serde_json::from_reader(BufReader::new(file))
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse context '{}': {}", name, e)))
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
        
        // Create fresh context
        let new_context = Context {
            name: self.state.current_context.clone(),
            messages: Vec::new(),
            created_at: now_timestamp(),
            updated_at: 0,
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
}
