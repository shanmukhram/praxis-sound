//! The system prompt: built from the guide in `docs/agents/praxis-skill/` (the same knowledge
//! outside agents get) plus the catalogue tables generated from code. It is the stable, cached
//! prefix of every request, so it holds nothing that changes between requests: no time, no
//! sound state, no machine-specific availability (those go into the user message or tool
//! results).

use super::catalogue;

/// The assistant's role, method and tools.
pub const ASSISTANT_GUIDE: &str = include_str!("../../../../docs/agents/praxis-skill/assistant.md");
/// The guide written for outside agents; the prompt takes the sections in [`SKILL_SECTIONS`].
pub const SKILL_GUIDE: &str = include_str!("../../../../docs/agents/praxis-skill/SKILL.md");

/// The `SKILL.md` sections the assistant gets (headings without `## `).
pub const SKILL_SECTIONS: [&str; 5] = [
    "Writing a plan",
    "Reading the result",
    "Fixing failures",
    "Taste rules (the owner's; follow them by default)",
    "Changing a sound",
];

/// The text of the `## <heading>` section of a Markdown document (heading included), up to the
/// next `## ` heading. `None` when there is no such section.
pub fn section(doc: &str, heading: &str) -> Option<String> {
    let mut out = String::new();
    let mut inside = false;
    for line in doc.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if inside {
                break;
            }
            inside = h.trim() == heading;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// The whole system prompt.
pub fn system_prompt() -> String {
    // The guide's first paragraphs describe the file for people; the prompt starts at "You are".
    let guide = ASSISTANT_GUIDE
        .find("You are the sound assistant")
        .map_or(ASSISTANT_GUIDE, |i| &ASSISTANT_GUIDE[i..]);
    let mut out = String::from(guide.trim_end());
    out.push_str(
        "\n\n# Reference (from the Praxis guide for agents)\n\n\
         The sections below were written for agents that drive Praxis through its MCP tools \
         (`praxis_compose`, `praxis_edit`, ...). You have the tools listed above instead: \
         `compose_sound` takes the same ScorePlan, `compose_edit` the same EditPlan, \
         `render_check` returns the same checks.\n\n",
    );
    for h in SKILL_SECTIONS {
        if let Some(s) = section(SKILL_GUIDE, h) {
            out.push_str(s.trim_end());
            out.push_str("\n\n");
        }
    }
    out.push_str(&catalogue::prompt_tables());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_has_every_part_and_is_stable() {
        for h in SKILL_SECTIONS {
            assert!(section(SKILL_GUIDE, h).is_some(), "SKILL.md lost `## {h}`");
        }
        let p = system_prompt();
        assert!(p.starts_with("You are the sound assistant"));
        assert!(p.contains("## Taste rules"));
        assert!(p.contains("Rhythm first"));
        assert!(p.contains("## Presets and their parameters"));
        assert!(p.contains("`felt_piano`"));
        assert_eq!(p, system_prompt(), "the cached prefix must not vary");
        // Nothing machine-specific in the cached prefix (the sound folder lives under it).
        assert!(!p.contains("PRAXIS_HOME"));
        assert!(!p.contains(&crate::ops::praxis_home().display().to_string()));
    }

    #[test]
    fn sections_stop_at_the_next_heading() {
        let doc = "# T\n## A\na\n### sub\nb\n## B\nc\n";
        assert_eq!(section(doc, "A").unwrap(), "## A\na\n### sub\nb\n");
        assert_eq!(section(doc, "B").unwrap(), "## B\nc\n");
        assert!(section(doc, "C").is_none());
    }
}
