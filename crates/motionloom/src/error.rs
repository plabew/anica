// =========================================
// =========================================
// crates/motionloom/src/error.rs

fn format_graph_parse_error(line: usize, message: &str) -> String {
    if line > 0 {
        format!("line {line}: {message}")
    } else {
        message.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", format_graph_parse_error(*.line, &.message))]
pub struct GraphParseError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RuntimeCompileError {
    pub message: String,
}
