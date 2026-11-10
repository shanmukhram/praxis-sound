//! praxis-planner: turns briefs into Scores and edit requests into scoped patches.
//!
//! The model never produces audio. It fills a schema-constrained plan through one tool call;
//! the plan is validated, repaired on failure, and expanded into a Score by a deterministic
//! arranger. See docs/contracts/planner.md and docs/adr/0006-two-level-planner.md.

#![forbid(unsafe_code)]

// ---- frozen contract (orchestrator) ----
pub mod provider;

// ---- WP-P planner: implementation modules ----
pub mod arrange;
pub mod brief;
pub mod catalogue;
#[cfg(feature = "claude")]
pub mod claude;
pub mod edit;
pub mod motifs;
pub mod offline;
pub mod plan;
pub mod repair;
pub mod variations;

pub use provider::{Message, Provider, ProviderError, Role, ToolRequest};

pub use brief::{PlanOutcome, compose_plan, plan_brief};
pub use catalogue::Catalogue;
pub use offline::RuleProvider;
pub use plan::{ScorePlan, plan_schema};
pub use repair::{PlanError, Repair};
