//! `generate`: a brief (through the offline rule planner or Claude) or an agent-written
//! ScorePlan (`compose_plan`, no provider and no key) -> a `.praxis` project, optionally
//! rendered and checked.

use crate::error::{ErrorCode, OpError};
use crate::ops::key::resolve_key;
use crate::ops::render::{RenderSettings, render_score};
use crate::ops::{score_summary, to_values, write_project};
use praxis_planner::claude::{ClaudeConfig, ClaudeProvider, UreqTransport};
use praxis_planner::{
    Catalogue, PlanError, PlanOutcome, Provider, ProviderError, RuleProvider, compose_plan,
    plan_brief,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Overrides the Messages API URL (a proxy, or a local stand-in in tests).
pub const CLAUDE_URL_ENV: &str = "PRAXIS_CLAUDE_URL";

/// Seed used when none is given, so the same brief gives the same project.
pub const DEFAULT_SEED: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderChoice {
    Offline,
    Claude,
}

impl ProviderChoice {
    pub fn parse(s: &str) -> Option<ProviderChoice> {
        match s {
            "offline" => Some(ProviderChoice::Offline),
            "claude" => Some(ProviderChoice::Claude),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GenerateRequest {
    /// Plain-language brief. Exactly one of `prompt` and `plan`.
    #[serde(default)]
    pub prompt: Option<String>,
    /// A ScorePlan (see `praxis schema`, field `plan`) written by the caller.
    #[serde(default)]
    pub plan: Option<Value>,
    /// `None`: Claude when a key is available, else offline (and the payload says so).
    #[serde(default)]
    pub provider: Option<ProviderChoice>,
    /// Claude model override.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// The `.praxis` project to write.
    pub out: PathBuf,
    /// Also render the Score to this file (and check it).
    #[serde(default)]
    pub render: Option<PathBuf>,
    #[serde(default)]
    pub render_settings: RenderSettings,
}

fn default_seed() -> u64 {
    DEFAULT_SEED
}

fn plan_error(e: PlanError, direct: bool) -> OpError {
    match e {
        PlanError::Provider(ProviderError::NoApiKey) => {
            OpError::new(ErrorCode::NoApiKey, ProviderError::NoApiKey.to_string())
        }
        PlanError::Provider(p) => OpError::new(ErrorCode::ProviderFailed, p.to_string()),
        PlanError::Invalid(issues) => {
            let code = if direct {
                ErrorCode::InvalidScore
            } else {
                ErrorCode::ProviderFailed
            };
            let msg = PlanError::Invalid(issues.clone()).to_string();
            OpError::new(code, msg).with_issues(to_values(&issues))
        }
        other => OpError::new(ErrorCode::Error, other.to_string()),
    }
}

fn claude_provider(model: Option<&str>) -> ClaudeProvider {
    let mut config = ClaudeConfig::default();
    if let Some(m) = model {
        config.model = m.to_string();
    }
    if let Ok(url) = std::env::var(CLAUDE_URL_ENV)
        && !url.trim().is_empty()
    {
        config.url = url.trim().to_string();
    }
    ClaudeProvider::with_parts(
        config,
        Box::new(UreqTransport::default()),
        Box::new(resolve_key),
    )
}

pub fn generate(req: &GenerateRequest) -> Result<Value, OpError> {
    let catalogue = Catalogue::standard();
    let mut note: Option<String> = None;
    let outcome: PlanOutcome = match (&req.prompt, &req.plan) {
        (Some(_), Some(_)) => {
            return Err(OpError::new(
                ErrorCode::Error,
                "give a prompt or a plan, not both",
            ));
        }
        (None, None) => {
            return Err(OpError::new(
                ErrorCode::Error,
                "give a prompt (--prompt) or a plan (--plan)",
            ));
        }
        (None, Some(plan)) => {
            compose_plan(plan, &catalogue, req.seed).map_err(|e| plan_error(e, true))?
        }
        (Some(prompt), None) => {
            let choice = match req.provider {
                Some(c) => c,
                None if resolve_key().is_ok() => ProviderChoice::Claude,
                None => {
                    note = Some(
                        "no Anthropic API key found (ANTHROPIC_API_KEY or `praxis key set`), so \
                         the offline rule planner was used"
                            .into(),
                    );
                    ProviderChoice::Offline
                }
            };
            let provider: Box<dyn Provider> = match choice {
                ProviderChoice::Offline => Box::new(RuleProvider::new()),
                ProviderChoice::Claude => {
                    if resolve_key().is_err() {
                        return Err(OpError::new(
                            ErrorCode::NoApiKey,
                            ProviderError::NoApiKey.to_string(),
                        ));
                    }
                    Box::new(claude_provider(req.model.as_deref()))
                }
            };
            plan_brief(prompt, provider.as_ref(), &catalogue, req.seed)
                .map_err(|e| plan_error(e, false))?
        }
    };

    write_project(&req.out, &outcome.score)?;
    let render = match &req.render {
        Some(path) => Some(render_score(&outcome.score, path, &req.render_settings)?),
        None => None,
    };
    let mut v = json!({
        "project": req.out.display().to_string(),
        "score_summary": score_summary(&outcome.score),
        "provider": outcome.provider_id,
        "provider_note": note,
        "attempts": outcome.attempts,
        "repairs": outcome.repairs,
        "rounds": outcome.rounds,
        "lint": outcome.lint,
        "planner_notes": outcome.plan.notes,
        "plan": outcome.plan,
        "seed": req.seed,
    });
    if let Some(r) = render {
        v["render"] = r;
    }
    Ok(v)
}
