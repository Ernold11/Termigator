//! Termigator core: everything that does not need Qt.
//!
//! Module layout follows ARCHITECTURE.md §2; each module names the task that implements it.

#![forbid(unsafe_code)]

pub mod actions;
pub mod bulk;
pub mod config;
pub mod connectivity;
pub mod credentials;
pub mod export;
pub mod hosttree;
pub mod import;
pub mod konsole;
pub mod multiexec;
pub mod panes;
pub mod paths;
pub mod platform;
pub mod proc;
pub mod proto;
pub mod quick_history;
pub mod remote;
pub mod session_state;
pub mod shell;
pub mod single_instance;
pub mod text;
pub mod update_check;
pub mod vault;
