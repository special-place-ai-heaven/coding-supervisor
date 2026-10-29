//! MODE: enforcing at SubagentStop, Stop and PreToolUse; advisory/audit at PostToolUse (foreground only) and at the acceptance moment (no hook event exists). The gate's own state is kept outside the project tree when the environment allows, writes to it are denied and a hash chain detects edits; this raises the bar and detects tampering, it does not make the adapter enforcing against an adversarial agent holding a shell as the same OS user. Measured on Claude Code 2.1.312, 2026-10-29.
//!
//! Claude Code hook adapter for the completion gate: one hook payload in, at most one decision out.
#![forbid(unsafe_code)]

pub mod adapter;
pub mod candidate;
mod chain;
pub mod claim;
pub mod guard;
pub mod header;
pub mod location;
pub mod sha256;
mod state;
