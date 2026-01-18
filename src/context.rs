use serde::{Deserialize, Serialize};
use std::io::{self, ErrorKind};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Context {
    pub name: String,
    pub messages: Vec<Message>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextState {
    pub contexts: Vec<String>,
    pub current_context: String,
}

impl ContextState {
    pub fn switch_context(&mut self, name: String) -> io::Result<()> {
        validate_context_name(&name)?;
        self.current_context = name;
        Ok(())
    }
    
    pub fn save(&self, state_path: &PathBuf) -> io::Result<()> {
        use std::fs::OpenOptions;
        use std::io::BufWriter;
        
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

pub fn is_valid_context_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn validate_context_name(name: &str) -> io::Result<()> {
    if !is_valid_context_name(name) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid context name '{}'. Names must be alphanumeric with dashes and underscores only.", name),
        ));
    }
    Ok(())
}

pub fn now_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
