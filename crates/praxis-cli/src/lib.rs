//! praxis-cli: the shared command layer behind the `praxis` binary (FR-33) and, next, the MCP
//! server (WP-X, ADR-0010).
//!
//! Every command is a function in [`ops`] that takes a typed request (which also deserialises
//! from the JSON an MCP tool call carries) and returns the command's payload as JSON, or an
//! [`OpError`] with a stable code from docs/contracts/cli.md. The binary is a thin clap layer
//! over these functions, so the CLI and the MCP server can never drift apart.
//!
//! ```no_run
//! use praxis_cli::ops::{CheckRequest, check};
//! let payload = check(&CheckRequest {
//!     audio: "out.wav".into(),
//!     score: "song.praxis".into(),
//! })?;
//! assert!(payload["passed"].is_boolean());
//! # Ok::<(), praxis_cli::OpError>(())
//! ```

#![forbid(unsafe_code)]

pub mod error;
pub mod ops;
pub mod output;

pub use error::{ErrorCode, OpError};
pub use output::{emit, envelope, exit_code};
