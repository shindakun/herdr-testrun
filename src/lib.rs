//! herdr-testrun: a Herdr plugin that runs a project's tests in a pane, lists
//! the failures, and sends them to the workspace's agent. The library holds
//! everything; `main.rs` only dispatches argv.

pub mod adapters;
pub mod cli;
pub mod config;
pub mod detect;
pub mod herdr;
pub mod job;
pub mod model;
pub mod prompt;
pub mod runner;
pub mod sock;
pub mod state;
#[cfg(test)]
pub mod testutil;
pub mod tui;
