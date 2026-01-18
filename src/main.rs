
use serde::{Deserialize, Serialize};
use dirs_next::home_dir;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, ErrorKind, Write, BufRead};
use std::path::PathBuf;
use reqwest::{Client, StatusCode};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use futures_util::stream::StreamExt;
use tokio::io::{AsyncWriteExt, stdout};

const API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Debug)]
struct Cli {
    switch: Option<String>,
    list: bool,
    which: bool,
    delete: Option<String>,
    clear: bool,
    compact: bool,
    rename: Option<(String, String)>,
    history: bool,
    num_messages: Option<usize>,
    prompt: Vec<String>,
}

impl Cli {
    fn parse() -> io::Result<Self> {
        let args: Vec<String> = std::env::args().collect();
        
        let mut switch = None;
        let mut list = false;
        let mut which = false;
        let mut delete = None;
        let mut clear = false;
        let mut compact = false;
        let mut rename = None;
        let mut history = false;
        let mut num_messages: Option<usize> = None;
        let mut prompt = Vec::new();
        let mut i = 1;
        let mut is_prompt = false;
        
        while i < args.len() {
            let arg = &args[i];
            
            if is_prompt {
                prompt.push(arg.clone());
                i += 1;
                continue;
            }
            
            if arg == "--" {
                is_prompt = true;
                i += 1;
                continue;
            }
            
            if arg == "-s" || arg == "--switch" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                switch = Some(args[i + 1].clone());
                i += 2;
                continue;
            }
            
            if arg == "-l" || arg == "--list" {
                list = true;
                i += 1;
                continue;
            }
            
            if arg == "-w" || arg == "--which" {
                which = true;
                i += 1;
                continue;
            }
            
            if arg == "-d" || arg == "--delete" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                delete = Some(args[i + 1].clone());
                i += 2;
                continue;
            }
            
            if arg == "-C" || arg == "--clear" {
                clear = true;
                i += 1;
                continue;
            }
            
            if arg == "-c" || arg == "--compact" {
                compact = true;
                i += 1;
                continue;
            }
            
            if arg == "-r" || arg == "--rename" {
                if i + 2 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires two arguments", arg)));
                }
                rename = Some((args[i + 1].clone(), args[i + 2].clone()));
                i += 3;
                continue;
            }
            
            if arg == "-H" || arg == "--history" {
                history = true;
                i += 1;
                continue;
            }
            
            if arg == "-n" || arg == "--num-messages" {
                if i + 1 >= args.len() {
                    return Err(io::Error::new(ErrorKind::InvalidInput, format!("{} requires an argument", arg)));
                }
                num_messages = Some(args[i + 1].parse().map_err(|_| {
                    io::Error::new(ErrorKind::InvalidInput, format!("Invalid number: {}", args[i + 1]))
                })?);
                i += 2;
                continue;
            }
            
            if arg == "-h" || arg == "--help" {
                Self::print_help();
                std::process::exit(0);
            }
            
            if arg == "-v" || arg == "--version" {
                println!("chibi {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            
            // Check if it starts with a dash
            if arg.starts_with('-') {
                return Err(io::Error::new(ErrorKind::InvalidInput, format!("Unknown option: {}", arg)));
            }
            
            // This is the start of the prompt
            is_prompt = true;
            prompt.push(arg.clone());
            i += 1;
        }
        
        // -n implies -H
        if num_messages.is_some() {
            history = true;
        }
        
        // Validate argument combinations
        let commands = [switch.is_some(), list, which, delete.is_some(), clear, compact, rename.is_some(), history]
            .iter()
            .filter(|&&x| x)
            .count();
        
        if commands > 1 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Only one command can be specified at a time"));
        }
        
        if commands > 0 && !prompt.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Cannot specify both a command and a prompt"));
        }
        
        Ok(Cli {
            switch,
            list,
            which,
            delete,
            clear,
            compact,
            rename,
            history,
            num_messages,
            prompt,
        })
    }
    
    fn print_help() {
        println!("chibi - A CLI tool for chatting with AI via OpenRouter");
        println!();
        println!("Usage:");
        println!("  chibi [OPTIONS] [PROMPT]");
        println!("  chibi [COMMAND]");
        println!();
        println!("Commands:");
        println!("  -s, --switch <NAME>     Switch to a different context");
        println!("  -l, --list              List all contexts");
        println!("  -w, --which             Show current context name");
        println!("  -d, --delete <NAME>     Delete a context");
        println!("  -C, --clear             Clear current context");
        println!("  -c, --compact           Compact current context");
        println!("  -r, --rename <OLD> <NEW>  Rename a context");
        println!("  -H, --history           Show recent messages (default: 6)");
        println!("  -n, --num-messages <N>  Number of messages to show (0 = all, implies -H)");
        println!();
        println!("Prompt input:");
        println!("  If arguments are provided after options, they are joined as the prompt.");
        println!("  Use -- to force the rest to be a prompt (e.g., chibi -- -this starts with dash)");
        println!("  If no arguments, read prompt from stdin (end with . on empty line)");
        println!();
        println!("Examples:");
        println!("  chibi What is Rust?");
        println!("  chibi -s coding write a function");
        println!("  chibi -- -this prompt starts with dash");
        println!("  chibi -l");
        println!("  chibi -r old-name new-name");
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Context {
    name: String,
    messages: Vec<Message>,
    created_at: u64,
    updated_at: u64,
}

fn is_valid_context_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn validate_context_name(name: &str) -> io::Result<()> {
    if !is_valid_context_name(name) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid context name '{}'. Names must be alphanumeric with dashes and underscores only.", name),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContextState {
    contexts: Vec<String>,
    current_context: String,
}

impl ContextState {
    fn switch_context(&mut self, name: String) -> io::Result<()> {
        validate_context_name(&name)?;
        self.current_context = name;
        Ok(())
    }
    
    fn save(&self, state_path: &PathBuf) -> io::Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(state_path)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to save state: {}", e)))?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    api_key: String,
    model: String,
    context_window_limit: usize,
    warn_threshold_percent: f32,
    #[serde(default = "default_auto_compact")]
    auto_compact: bool,
    #[serde(default = "default_auto_compact_threshold")]
    auto_compact_threshold: f32,
    #[serde(default = "default_base_url")]
    base_url: String,
}

fn default_auto_compact() -> bool {
    false
}

fn default_auto_compact_threshold() -> f32 {
    80.0
}

fn default_base_url() -> String {
    API_URL.to_string()
}

#[derive(Debug)]
struct AppState {
    config: Config,
    state: ContextState,
    state_path: PathBuf,
    contexts_dir: PathBuf,
    prompts_dir: PathBuf,
}

impl AppState {
    fn load() -> io::Result<Self> {
        let home = home_dir().ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Home directory not found"))?;
        let chibi_dir = home.join(".chibi");
        let contexts_dir = chibi_dir.join("contexts");
        let prompts_dir = chibi_dir.join("prompts");
        
        // Create directories if they don't exist
        fs::create_dir_all(&chibi_dir)?;
        fs::create_dir_all(&contexts_dir)?;
        fs::create_dir_all(&prompts_dir)?;
        
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
        })
    }
    
    fn save(&self) -> io::Result<()> {
        self.state.save(&self.state_path)
    }
    
    fn context_dir(&self, name: &str) -> PathBuf {
        self.contexts_dir.join(name)
    }
    
    fn context_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("context.json")
    }
    
    fn transcript_file(&self, name: &str) -> PathBuf {
        self.context_dir(name).join("transcript.txt")
    }
    
    fn ensure_context_dir(&self, name: &str) -> io::Result<()> {
        let dir = self.context_dir(name);
        fs::create_dir_all(&dir)
    }
    
    fn load_context(&self, name: &str) -> io::Result<Context> {
        let file = File::open(self.context_file(name))?;
        serde_json::from_reader(BufReader::new(file))
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("Failed to parse context '{}': {}", name, e)))
    }
    
    fn save_context(&self, context: &Context) -> io::Result<()> {
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
    
    fn append_to_transcript(&self, context: &Context) -> io::Result<()> {
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
            writeln!(file, "=== {} ===", msg.role.to_uppercase())?;
            writeln!(file, "{}", msg.content)?;
            writeln!(file, "")?;
        }
        writeln!(file, "================================")?;
        writeln!(file, "")?;
        
        Ok(())
    }
    
    fn get_current_context(&self) -> io::Result<Context> {
        self.load_context(&self.state.current_context)
            .or_else(|e| {
                if e.kind() == ErrorKind::NotFound {
                    // Return empty context if it doesn't exist yet
                    Ok(Context {
                        name: self.state.current_context.clone(),
                        messages: Vec::new(),
                        created_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        updated_at: 0,
                    })
                } else {
                    Err(e)
                }
            })
    }
    
    fn save_current_context(&self, context: &Context) -> io::Result<()> {
        self.save_context(context)?;
        
        // Ensure the context is tracked in state
        if !self.state.contexts.contains(&context.name) {
            let mut new_state = self.state.clone();
            new_state.contexts.push(context.name.clone());
            new_state.save(&self.state_path)?;
        }
        
        Ok(())
    }
    
    fn add_message(&self, context: &mut Context, role: String, content: String) {
        context.messages.push(Message { role, content });
        context.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    
    fn clear_context(&self) -> io::Result<()> {
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
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            updated_at: 0,
        };
        
        self.save_current_context(&new_context)?;
        Ok(())
    }
    
    fn delete_context(&self, name: &str) -> io::Result<bool> {
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
    
    fn rename_context(&self, old_name: &str, new_name: &str) -> io::Result<()> {
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
    
    fn list_contexts(&self) -> Vec<String> {
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
    
    fn calculate_token_count(&self, messages: &[Message]) -> usize {
        // Rough estimation: 4 chars per token on average
        messages.iter().map(|m| (m.content.len() + m.role.len()) / 4).sum()
    }
    
    fn should_warn(&self, messages: &[Message]) -> bool {
        let tokens = self.calculate_token_count(messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.warn_threshold_percent
    }
    
    fn remaining_tokens(&self, messages: &[Message]) -> usize {
        let tokens = self.calculate_token_count(messages);
        if tokens >= self.config.context_window_limit {
            0
        } else {
            self.config.context_window_limit - tokens
        }
    }
    
    fn load_prompt(&self, name: &str) -> io::Result<String> {
        let prompt_path = self.prompts_dir.join(format!("{}.md", name));
        if prompt_path.exists() {
            fs::read_to_string(&prompt_path)
        } else {
            Ok(String::new())
        }
    }
    
    fn should_auto_compact(&self, context: &Context) -> bool {
        if !self.config.auto_compact {
            return false;
        }
        let tokens = self.calculate_token_count(&context.messages);
        let usage_percent = (tokens as f32 / self.config.context_window_limit as f32) * 100.0;
        usage_percent >= self.config.auto_compact_threshold
    }
}

async fn compact_context_with_llm_internal(app: &AppState, print_message: bool) -> io::Result<()> {
    let context = app.get_current_context()?;
    
    if context.messages.is_empty() {
        if print_message {
            println!("Context is already empty");
        }
        return Ok(());
    }
    
    if context.messages.len() <= 2 {
        // Nothing to compact
        if print_message {
            println!("Context is already compact (2 or fewer messages)");
        }
        return Ok(());
    }
    
    // Append to transcript before compacting
    app.append_to_transcript(&context)?;
    
    if print_message {
        eprintln!("[Compacting] Messages: {} -> requesting summary...", context.messages.len());
    }
    
    let client = Client::new();
    
    // Load compaction prompt
    let compaction_prompt = app.load_prompt("compaction")?;
    let default_compaction_prompt = "Please summarize the following conversation into a concise summary. Capture the key points, decisions, and context.";
    let compaction_prompt = if compaction_prompt.is_empty() {
        eprintln!("[WARN] No compaction prompt found at ~/.chibi/prompts/compaction.md. Using default.");
        default_compaction_prompt
    } else {
        &compaction_prompt
    };
    
    // Build conversation text for summarization
    let mut conversation_text = String::new();
    for m in &context.messages {
        if m.role == "system" {
            continue;
        }
        conversation_text.push_str(&format!("[{}]: {}\n\n", m.role.to_uppercase(), m.content));
    }
    
    // Prepare messages for compaction request - use a single user message with the conversation
    let compaction_messages = vec![
        serde_json::json!({
            "role": "system",
            "content": compaction_prompt,
        }),
        serde_json::json!({
            "role": "user",
            "content": format!("Please summarize this conversation:\n\n{}", conversation_text),
        }),
    ];
    
    let request_body = serde_json::json!({
        "model": app.config.model,
        "messages": compaction_messages,
        "stream": false,
    });
    
    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;
    
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("API error ({}): {}", status, body),
        ));
    }
    
    let json: serde_json::Value = response.json().await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to parse response: {}", e)))?;
    
    let summary = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["choices"][0]["content"].as_str())
        .unwrap_or_else(|| {
            eprintln!("[DEBUG] Response structure: {}", json);
            ""
        })
        .to_string();
    
    if summary.is_empty() {
        eprintln!("[DEBUG] Full response: {}", json);
        return Err(io::Error::new(
            ErrorKind::Other, 
            "Empty summary received from LLM. This can happen with free-tier models. Try again or use a different model."
        ));
    }
    
    // Prepare continuation prompt
    let continuation_prompt = app.load_prompt("continuation")?;
    let continuation_prompt = if continuation_prompt.is_empty() {
        "Here is a summary of the previous conversation. Continue from this point."
    } else {
        &continuation_prompt
    };
    
    // Load system prompt
    let system_prompt = app.load_prompt("chibi")?;
    
    // Create new context with system prompt, continuation instructions, and summary
    let mut new_context = Context {
        name: context.name.clone(),
        messages: Vec::new(),
        created_at: context.created_at,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };
    
    // Add system prompt as first message
    if !system_prompt.is_empty() {
        new_context.messages.push(Message {
            role: "system".to_string(),
            content: system_prompt.clone(),
        });
    }
    
    // Add continuation prompt + summary as user message
    new_context.messages.push(Message {
        role: "user".to_string(),
        content: format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
    });
    
    // Add assistant acknowledgment
    let messages = vec![
        serde_json::json!({
            "role": "system",
            "content": system_prompt,
        }),
        serde_json::json!({
            "role": "user",
            "content": format!("{}\n\n--- SUMMARY ---\n{}", continuation_prompt, summary),
        }),
    ];
    
    let request_body = serde_json::json!({
        "model": app.config.model,
        "messages": messages,
        "stream": false,
    });
    
    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;
    
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("API error ({}): {}", status, body),
        ));
    }
    
    let json: serde_json::Value = response.json().await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to parse response: {}", e)))?;
    
    let acknowledgment = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    
    new_context.messages.push(Message {
        role: "assistant".to_string(),
        content: acknowledgment,
    });
    
    // Save the new context
    app.save_current_context(&new_context)?;
    
    if print_message {
        println!("Context compacted (history saved to transcript)");
    }
    Ok(())
}

fn read_prompt_from_stdin() -> io::Result<String> {
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut buffer = String::new();
    let mut prompt = String::new();
    let mut first = true;
    
    loop {
        buffer.clear();
        let bytes_read = stdin_lock.read_line(&mut buffer)?;
        
        // EOF (Ctrl+D)
        if bytes_read == 0 {
            break;
        }
        
        // Remove trailing newline
        if buffer.ends_with('\n') {
            buffer.pop();
            if buffer.ends_with('\r') {
                buffer.pop();
            }
        }
        
        // Check for termination: a single dot on a line
        if buffer.trim() == "." {
            break;
        }
        
        if !first {
            prompt.push(' ');
        }
        prompt.push_str(&buffer);
        first = false;
    }
    
    Ok(prompt)
}

async fn send_prompt(app: &AppState, prompt: String) -> io::Result<()> {
    if prompt.trim().is_empty() {
        return Err(io::Error::new(ErrorKind::InvalidInput, "Prompt cannot be empty"));
    }
    
    let mut context = app.get_current_context()?;
    
    // Add user message
    app.add_message(&mut context, "user".to_string(), prompt.clone());
    
    // Check if we need to warn about context window
    if app.should_warn(&context.messages) {
        let remaining = app.remaining_tokens(&context.messages);
        eprintln!("[Context window warning: {} tokens remaining]", remaining);
    }
    
    // Auto-compaction check
    if app.should_auto_compact(&context) {
        return compact_context_with_llm(app).await;
    }
    
    // Prepare messages for API
    // Include system prompt if not already in context
    let system_prompt = app.load_prompt("chibi")?;
    let context_has_system = context.messages.iter().any(|m| m.role == "system");
    
    let mut messages: Vec<serde_json::Value> = if !system_prompt.is_empty() && !context_has_system {
        vec![serde_json::json!({
            "role": "system",
            "content": system_prompt,
        })]
    } else {
        Vec::new()
    };
    
    // Add conversation messages
    for m in &context.messages {
        messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content,
        }));
    }
    
    let request_body = serde_json::json!({
        "model": app.config.model,
        "messages": messages,
        "stream": true,
    });
    
    let client = Client::new();
    let response = client
        .post(&app.config.base_url)
        .header(AUTHORIZATION, format!("Bearer {}", app.config.api_key))
        .header(CONTENT_TYPE, "application/json")
        .body(request_body.to_string())
        .send()
        .await
        .map_err(|e| io::Error::new(ErrorKind::Other, format!("Failed to send request: {}", e)))?;
    
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("API error ({}): {}", status, body),
        ));
    }
    
    let mut stream = response.bytes_stream();
    let mut stdout = stdout();
    let mut full_response = String::new();
    
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| io::Error::new(ErrorKind::Other, format!("Stream error: {}", e)))?;
        let chunk_str = std::str::from_utf8(&chunk)
            .map_err(|e| io::Error::new(ErrorKind::Other, format!("UTF-8 error: {}", e)))?;
        
        // Parse Server-Sent Events format
        for line in chunk_str.lines() {
            if line.starts_with("data: ") {
                let data = &line[6..];
                if data == "[DONE]" {
                    continue;
                }
                
                let json: serde_json::Value = serde_json::from_str(data)
                    .map_err(|e| io::Error::new(ErrorKind::Other, format!("JSON parse error: {}", e)))?;
                
                if let Some(choices) = json["choices"].as_array() {
                    if let Some(choice) = choices.get(0) {
                        if let Some(delta) = choice.get("delta") {
                            if let Some(content) = delta["content"].as_str() {
                                full_response.push_str(content);
                                stdout.write_all(content.as_bytes()).await?;
                                stdout.flush().await?;
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Add assistant response
    app.add_message(&mut context, "assistant".to_string(), full_response);
    
    // Save the updated context
    app.save_current_context(&context)?;
    
    // Check context window after response
    if app.should_warn(&context.messages) {
        let remaining = app.remaining_tokens(&context.messages);
        eprintln!("[Context window warning: {} tokens remaining]", remaining);
    }
    
    println!();
    Ok(())
}

async fn compact_context_with_llm(app: &AppState) -> io::Result<()> {
    compact_context_with_llm_internal(app, false).await
}

async fn compact_context_with_llm_manual(app: &AppState) -> io::Result<()> {
    compact_context_with_llm_internal(app, true).await
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse()?;
    
    let mut app = AppState::load()?;
    
    if let Some(name) = cli.switch {
        app.state.switch_context(name)?;
        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                updated_at: 0,
            };
            app.save_current_context(&new_context)?;
        }
        app.save()?;
    } else if cli.list {
        let contexts = app.list_contexts();
        let current = &app.state.current_context;
        for name in contexts {
            if &name == current {
                println!("* {}", name);
            } else {
                println!("  {}", name);
            }
        }
    } else if cli.which {
        println!("{}", app.state.current_context);
    } else if let Some(name) = cli.delete {
        match app.delete_context(&name) {
            Ok(true) => println!("Deleted context: {}", name),
            Ok(false) => println!("Context '{}' not found", name),
            Err(e) => return Err(e),
        }
    } else if cli.clear {
        app.clear_context()?;
        println!("Context cleared (history saved to transcript)");
    } else if cli.compact {
        compact_context_with_llm_manual(&app).await?;
    } else if let Some((old_name, new_name)) = cli.rename {
        app.rename_context(&old_name, &new_name)?;
        println!("Renamed context '{}' to '{}'", old_name, new_name);
    } else if cli.history {
        let context = app.get_current_context()?;
        let num = cli.num_messages.unwrap_or(6);
        let messages: Vec<_> = if num == 0 {
            context.messages.iter().collect()
        } else {
            context.messages.iter().rev().take(num).collect::<Vec<_>>().into_iter().rev().collect()
        };
        for msg in messages {
            if msg.role == "system" {
                continue;
            }
            println!("=== {} ===", msg.role.to_uppercase());
            println!("{}", msg.content);
            println!();
        }
    } else if !cli.prompt.is_empty() {
        let prompt = cli.prompt.join(" ");
        // Create the context if it doesn't exist
        if !app.context_dir(&app.state.current_context).exists() {
            let new_context = Context {
                name: app.state.current_context.clone(),
                messages: Vec::new(),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                updated_at: 0,
            };
            app.save_current_context(&new_context)?;
        }
        send_prompt(&app, prompt).await?;
    } else {
        // No command and no prompt - read from stdin
        let prompt = read_prompt_from_stdin()?;
        if prompt.trim().is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Prompt cannot be empty"));
        }
        send_prompt(&app, prompt).await?;
    }
    
    Ok(())
}
