//! Session statistics tracking for the `/stats` command.
//!
//! Tracks metrics across the session including:
//! - Command execution counts and success/failure rates
//! - Files modified during the session
//! - Token usage breakdown by turn
//! - Time spent waiting for model vs executing tools

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use codex_core::protocol::TokenUsage;

/// Statistics for a single command execution.
#[derive(Debug, Clone)]
pub struct CommandStat {
    #[allow(dead_code)]
    pub command: String,
    pub exit_code: i32,
    pub duration: Duration,
}

impl CommandStat {
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Token usage for a single turn.
#[derive(Debug, Clone, Default)]
pub struct TurnTokenUsage {
    pub turn_number: u32,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[allow(dead_code)]
    pub reasoning_tokens: i64,
    #[allow(dead_code)]
    pub cached_tokens: i64,
}

impl TurnTokenUsage {
    pub fn total(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }
}

/// Aggregated session statistics.
#[derive(Debug, Clone)]
pub struct SessionStats {
    /// All commands executed during the session.
    commands: Vec<CommandStat>,

    /// Files that were modified (via apply_patch or similar).
    files_modified: HashMap<PathBuf, u32>,

    /// Files that were read.
    files_accessed: HashMap<PathBuf, u32>,

    /// Token usage per turn.
    turn_token_usage: Vec<TurnTokenUsage>,

    /// Current turn number (1-indexed).
    current_turn: u32,

    /// Total time spent waiting for model responses.
    model_wait_time: Duration,

    /// Total time spent executing tools (commands, file ops).
    tool_execution_time: Duration,

    /// When the current model request started (for tracking wait time).
    model_request_start: Option<Instant>,

    /// When the current tool execution started.
    tool_execution_start: Option<Instant>,

    /// Session start time.
    session_start: Instant,
}

impl SessionStats {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            files_modified: HashMap::new(),
            files_accessed: HashMap::new(),
            turn_token_usage: Vec::new(),
            current_turn: 0,
            model_wait_time: Duration::ZERO,
            tool_execution_time: Duration::ZERO,
            model_request_start: None,
            tool_execution_start: None,
            session_start: Instant::now(),
        }
    }

    // -------------------------------------------------------------------------
    // Command tracking
    // -------------------------------------------------------------------------

    /// Record a completed command execution.
    pub fn record_command(&mut self, command: String, exit_code: i32, duration: Duration) {
        self.commands.push(CommandStat {
            command,
            exit_code,
            duration,
        });
    }

    /// Get total number of commands executed.
    pub fn total_commands(&self) -> usize {
        self.commands.len()
    }

    /// Get number of successful commands (exit code 0).
    pub fn successful_commands(&self) -> usize {
        self.commands.iter().filter(|c| c.is_success()).count()
    }

    /// Get number of failed commands (non-zero exit code).
    pub fn failed_commands(&self) -> usize {
        self.commands.iter().filter(|c| !c.is_success()).count()
    }

    /// Get success rate as a percentage (0-100).
    pub fn success_rate(&self) -> f64 {
        if self.commands.is_empty() {
            return 100.0;
        }
        (self.successful_commands() as f64 / self.commands.len() as f64) * 100.0
    }

    /// Get total command execution time.
    pub fn total_command_time(&self) -> Duration {
        self.commands.iter().map(|c| c.duration).sum()
    }

    // -------------------------------------------------------------------------
    // File tracking
    // -------------------------------------------------------------------------

    /// Record a file modification.
    pub fn record_file_modified(&mut self, path: PathBuf) {
        *self.files_modified.entry(path).or_insert(0) += 1;
    }

    /// Record a file access (read).
    pub fn record_file_accessed(&mut self, path: PathBuf) {
        *self.files_accessed.entry(path).or_insert(0) += 1;
    }

    /// Get number of unique files modified.
    pub fn files_modified_count(&self) -> usize {
        self.files_modified.len()
    }

    /// Get number of unique files accessed.
    pub fn files_accessed_count(&self) -> usize {
        self.files_accessed.len()
    }

    /// Get the most frequently accessed files (sorted by access count).
    pub fn top_accessed_files(&self, limit: usize) -> Vec<(&PathBuf, u32)> {
        let mut files: Vec<_> = self.files_accessed.iter().map(|(k, v)| (k, *v)).collect();
        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(limit);
        files
    }

    /// Get the most frequently modified files (sorted by modification count).
    #[allow(dead_code)]
    pub fn top_modified_files(&self, limit: usize) -> Vec<(&PathBuf, u32)> {
        let mut files: Vec<_> = self.files_modified.iter().map(|(k, v)| (k, *v)).collect();
        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(limit);
        files
    }

    // -------------------------------------------------------------------------
    // Turn and token tracking
    // -------------------------------------------------------------------------

    /// Start a new turn.
    pub fn start_turn(&mut self) {
        self.current_turn += 1;
    }

    /// Record token usage for the current turn.
    pub fn record_turn_tokens(&mut self, usage: &TokenUsage) {
        let turn_usage = TurnTokenUsage {
            turn_number: self.current_turn,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            reasoning_tokens: usage.reasoning_output_tokens,
            cached_tokens: usage.cached_input_tokens,
        };
        self.turn_token_usage.push(turn_usage);
    }

    /// Get the current turn number.
    pub fn current_turn(&self) -> u32 {
        self.current_turn
    }

    /// Get token usage breakdown by turn.
    pub fn turn_token_breakdown(&self) -> &[TurnTokenUsage] {
        &self.turn_token_usage
    }

    /// Get total tokens across all turns.
    pub fn total_tokens(&self) -> i64 {
        self.turn_token_usage
            .iter()
            .map(TurnTokenUsage::total)
            .sum()
    }

    /// Get total input tokens.
    pub fn total_input_tokens(&self) -> i64 {
        self.turn_token_usage.iter().map(|t| t.input_tokens).sum()
    }

    /// Get total output tokens.
    pub fn total_output_tokens(&self) -> i64 {
        self.turn_token_usage.iter().map(|t| t.output_tokens).sum()
    }

    // -------------------------------------------------------------------------
    // Timing tracking
    // -------------------------------------------------------------------------

    /// Mark the start of a model request (for tracking wait time).
    pub fn start_model_request(&mut self) {
        self.model_request_start = Some(Instant::now());
    }

    /// Mark the end of a model request.
    pub fn end_model_request(&mut self) {
        if let Some(start) = self.model_request_start.take() {
            self.model_wait_time += start.elapsed();
        }
    }

    /// Mark the start of tool execution.
    pub fn start_tool_execution(&mut self) {
        self.tool_execution_start = Some(Instant::now());
    }

    /// Mark the end of tool execution.
    pub fn end_tool_execution(&mut self) {
        if let Some(start) = self.tool_execution_start.take() {
            self.tool_execution_time += start.elapsed();
        }
    }

    /// Get total time waiting for model responses.
    pub fn model_wait_time(&self) -> Duration {
        self.model_wait_time
    }

    /// Get total time executing tools.
    pub fn tool_execution_time(&self) -> Duration {
        self.tool_execution_time
    }

    /// Get total session duration.
    pub fn session_duration(&self) -> Duration {
        self.session_start.elapsed()
    }

    /// Get percentage of time spent waiting for model.
    pub fn model_wait_percentage(&self) -> f64 {
        let total = self.session_duration().as_secs_f64();
        if total == 0.0 {
            return 0.0;
        }
        (self.model_wait_time.as_secs_f64() / total) * 100.0
    }

    /// Get percentage of time spent executing tools.
    pub fn tool_execution_percentage(&self) -> f64 {
        let total = self.session_duration().as_secs_f64();
        if total == 0.0 {
            return 0.0;
        }
        (self.tool_execution_time.as_secs_f64() / total) * 100.0
    }
}

/// Format a duration for display.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{hours}h {mins}m")
    } else if secs >= 60 {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{mins}m {remaining_secs}s")
    } else if secs > 0 {
        format!("{secs}s")
    } else {
        let millis = d.as_millis();
        format!("{millis}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_tracking() {
        let mut stats = SessionStats::new();

        stats.record_command("ls".to_string(), 0, Duration::from_secs(1));
        stats.record_command("cat file.txt".to_string(), 0, Duration::from_secs(2));
        stats.record_command("grep pattern".to_string(), 1, Duration::from_secs(1));

        assert_eq!(stats.total_commands(), 3);
        assert_eq!(stats.successful_commands(), 2);
        assert_eq!(stats.failed_commands(), 1);
        assert!((stats.success_rate() - 66.666).abs() < 1.0);
    }

    #[test]
    fn test_file_tracking() {
        let mut stats = SessionStats::new();

        stats.record_file_accessed(PathBuf::from("src/main.rs"));
        stats.record_file_accessed(PathBuf::from("src/main.rs"));
        stats.record_file_accessed(PathBuf::from("src/lib.rs"));
        stats.record_file_modified(PathBuf::from("src/main.rs"));

        assert_eq!(stats.files_accessed_count(), 2);
        assert_eq!(stats.files_modified_count(), 1);

        let top = stats.top_accessed_files(5);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].1, 2); // main.rs accessed twice
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_millis(500)), "500ms");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
    }
}
