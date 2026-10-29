//! MODE: enforcing at SubagentStop, Stop and PreToolUse; advisory/audit at PostToolUse (foreground only) and at the acceptance moment (no hook event exists). Measured on Claude Code 2.1.312, 2026-10-29.
//!
//! Claude Code hook adapter for the completion gate: one hook payload in, at most one decision out.
#![forbid(unsafe_code)]

pub mod adapter;
pub mod candidate;
pub mod claim;
pub mod header;
pub mod sha256;
mod state;
