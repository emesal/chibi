     Configuration Update Implementation Plan

     Overview
     Restructure configuration system to support:
     - Per-context overrides via local.toml
     - Model metadata and aliases via models.toml
     - Username configuration at multiple levels
     - Context lockfiles for multi-process safety
     - Noop recursion tool replacing built-in continue_processing

     1. File Structure Changes

     New Files
     ~/.chibi/
       ├── models.toml           # Model metadata and aliases
       ├── models.example.toml   # Example with common models
       └── contexts/
           └── <name>/
               ├── local.toml      # Per-context overrides
               └── .lock/           # Context lockfile directory
                   └── lockfile      # Contains "PID timestamp"

     2. Models.toml Structure

     # ~/.chibi/models.toml
     [models]
     [models.quick]
     name = "xiaomi/mimo-v2-flash:free"
     context_window = 262144
     pricing_input = 0.00000001
     pricing_output = 0.00000002

     [models.smart]
     name = "anthropic/claude-3.5-sonnet"
     context_window = 200000
     pricing_input = 0.000003
     pricing_output = 0.000015

     3. Config Priority Hierarchy

     CLI flags (-u, -U, temp overrides)
         ↓
     local.toml overrides
         ↓
     config.toml defaults
         ↓
     models.toml (for model expansion)
         ↓
     Rust binary defaults

     4. New Config Structures

     pub struct ModelMetadata {
         pub name: String,
         pub context_window: usize,
         pub pricing_input: Option<f64>,
         pub pricing_output: Option<f64>,
     }

     pub struct LocalConfig {
         pub model: Option<String>,
         pub api_key: Option<String>,
         pub username: Option<String>,
         pub auto_compact: Option<bool>,
         pub reflection_enabled: Option<bool>,
         pub max_recursion_depth: Option<usize>,
     }

     5. Lockfile Implementation

     pub struct ContextLock {
         lock_dir: PathBuf,
     }

     impl ContextLock {
         pub fn acquire(context_dir: &PathBuf, heartbeat_secs: u64, timeout_secs: u64) -> io::Result<Self>;
         pub fn touch(&self) -> io::Result<()>;
         pub fn release(self) -> io::Result<()>;
         pub fn is_stale(lock_dir: &PathBuf, stale_multiplier: f32) -> bool;
     }

     Behavior:
     - Acquire at process start (RAII auto-release)
     - Background thread updates timestamp every lock_heartbeat_seconds
     - Stale after 1.5x heartbeat duration
     - Cleanup on acquisition if stale

     6. CLI Changes

     New flags:
     pub username: Option<String>,      // -u, --username (persistent to local.toml)
     pub temp_username: Option<String>, // -U, --temp-username (this invocation only)

     7. Recursion Tool

     Remove built-in:
     - CONTINUE_TOOL_NAME, ContinueSignal, check_continue_signal()

     Create external tool examples/tools/recurse:
     # Noop: returns note as JSON
     echo "{\"note\": \"$note\"}"

     Main loop change:
     if tc.name == "recurse" {
         let note = args["note"].as_str().unwrap_or("");
         next_prompt = format!("[Continuing from previous round]\n\nNote to self: {}", note);
         break; // Continue to next LLM turn
     }

     8. Sub-Agent Tool

     Rename agent → sub-agent

     Simplify schema:
     {
       name: sub-agent,
       parameters: {
         context: {type: string},
         task: {type: string},
         system_prompt: {type: string}
       },
       required: [context, task]
     }

     Remove: mode parameter and continue mode logic.
