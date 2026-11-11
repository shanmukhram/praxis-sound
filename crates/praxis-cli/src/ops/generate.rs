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

/// Overrides the Messages API URL with a local stand-in (tests). Only loopback hosts are
/// accepted ([`claude_url_override`]).
pub const CLAUDE_URL_ENV: &str = "PRAXIS_CLAUDE_URL";

/// The Messages API address from [`CLAUDE_URL_ENV`], when set. It exists for tests (a local
/// stand-in for the API), so it may only name this machine: `http(s)://` and a host of
/// `localhost`, `127.0.0.1` (any `127.x.y.z`) or `[::1]`, with an optional port and path.
/// Anything else is refused, so an environment variable can never send the API key to another
/// server.
pub fn claude_url_override() -> Result<Option<String>, OpError> {
    match std::env::var(CLAUDE_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => {
            let url = url.trim();
            if is_loopback_url(url) {
                Ok(Some(url.to_string()))
            } else {
                Err(OpError::new(
                    ErrorCode::Error,
                    format!(
                        "{CLAUDE_URL_ENV} must point at this machine (localhost, 127.0.0.1 or \
                         [::1]), not `{url}`: it is only for tests, and the API key is never \
                         sent anywhere but the Anthropic API. Unset it to use the API"
                    ),
                ))
            }
        }
        _ => Ok(None),
    }
}

/// True for an `http://` or `https://` URL whose host is a loopback address or `localhost`
/// (and no user info, percent-escapes or backslashes, which could hide another host).
pub fn is_loopback_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains(['@', '\\', '%']) {
        return false;
    }
    let port = if let Some(v6) = authority.strip_prefix('[') {
        let Some((host, after)) = v6.split_once(']') else {
            return false;
        };
        if !host
            .parse::<std::net::Ipv6Addr>()
            .is_ok_and(|ip| ip.is_loopback())
        {
            return false;
        }
        match after {
            "" => None,
            p => match p.strip_prefix(':') {
                Some(p) => Some(p),
                None => return false,
            },
        }
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        let local = host == "localhost"
            || host
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !local {
            return false;
        }
        port
    };
    port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()))
}

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

fn claude_provider(model: Option<&str>) -> Result<ClaudeProvider, OpError> {
    let mut config = ClaudeConfig::default();
    if let Some(m) = model {
        config.model = m.to_string();
    }
    if let Some(url) = claude_url_override()? {
        config.url = url;
    }
    Ok(ClaudeProvider::with_parts(
        config,
        Box::new(UreqTransport::default()),
        Box::new(resolve_key),
    ))
}

/// The planner for a request: the one asked for, else Claude when a key is available, else the
/// offline rules (with a note saying so).
pub(crate) fn choose_provider(
    choice: Option<ProviderChoice>,
    model: Option<&str>,
) -> Result<(Box<dyn Provider>, Option<String>), OpError> {
    let mut note = None;
    let choice = match choice {
        Some(c) => c,
        None if resolve_key().is_ok() => ProviderChoice::Claude,
        None => {
            note = Some(
                "no Anthropic API key found (ANTHROPIC_API_KEY or `praxis key set`), so the \
                 offline rule planner was used"
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
            Box::new(claude_provider(model)?)
        }
    };
    Ok((provider, note))
}

pub(crate) fn plan_error_of(e: PlanError) -> OpError {
    plan_error(e, false)
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
            let (provider, provider_note) = choose_provider(req.provider, req.model.as_deref())?;
            note = provider_note;
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

#[cfg(test)]
mod url_tests {
    use super::is_loopback_url;

    #[test]
    fn only_loopback_addresses_may_stand_in_for_the_api() {
        for ok in [
            "http://127.0.0.1:8080/v1/messages",
            "http://localhost/v1/messages",
            "https://LOCALHOST:443",
            "http://[::1]:9000/v1/messages",
            "http://127.0.0.1",
        ] {
            assert!(is_loopback_url(ok), "{ok}");
        }
        for bad in [
            "https://example.com",
            "https://api.anthropic.com/v1/messages",
            "http://localhost@example.com/",
            "http://localhost.example.com/",
            "http://127.0.0.1.example.com/",
            "http://example.com/127.0.0.1",
            "ftp://127.0.0.1/",
            "http://[::2]/",
            "http://localhost:80x/",
            "http://localhost%2eexample.com/",
            "127.0.0.1:8080",
            "",
        ] {
            assert!(!is_loopback_url(bad), "{bad}");
        }
    }
}
