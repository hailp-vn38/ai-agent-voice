//! Default agent system prompts and Built-in Job Skills seeded into the workspace if absent.
//!
//! Canonical prompt constants are shared from [`orchestrator_flow_rs::prompts`], while the 7
//! Built-in Job Skills live in `crates/voice-agent-server/builtin-skills/<id>/SKILL.md` so the
//! server can seed runtime `skills/` and restore defaults on demand.

use std::path::Path;

pub use orchestrator_flow_rs::{
    DAILY_ROOT_PROMPT, MEDIA_CREATOR_PROMPT, MEDIA_PROCESSOR_PROMPT, REMINDER_PLANNER_PROMPT,
    REPORT_STAGER_PROMPT, SOURCE_COLLECTOR_PROMPT, WIKI_CURATOR_PROMPT, WIKI_INGESTOR_PROMPT,
    WIKI_RESEARCHER_PROMPT, WIKI_ROOT_PROMPT,
};

pub const WIKI_TAXONOMY_SKILL: &str =
    include_str!("../../../builtin-skills/wiki-taxonomy/SKILL.md");
pub const WIKI_INGEST_SKILL: &str = include_str!("../../../builtin-skills/wiki-ingest/SKILL.md");
pub const WIKI_QUERY_SKILL: &str = include_str!("../../../builtin-skills/wiki-query/SKILL.md");
pub const WIKI_LINT_SKILL: &str = include_str!("../../../builtin-skills/wiki-lint/SKILL.md");
pub const DAILY_SYNTHESIS_STAGING_SKILL: &str =
    include_str!("../../../builtin-skills/daily-synthesis-staging/SKILL.md");
pub const DAILY_TASK_REMINDER_SKILL: &str =
    include_str!("../../../builtin-skills/daily-task-reminder/SKILL.md");
pub const TRADING_CHART_SKILL: &str =
    include_str!("../../../builtin-skills/trading-chart/SKILL.md");

pub const BUILTIN_SKILLS: &[(&str, &str)] = &[
    ("wiki-taxonomy", WIKI_TAXONOMY_SKILL),
    ("wiki-ingest", WIKI_INGEST_SKILL),
    ("wiki-query", WIKI_QUERY_SKILL),
    ("wiki-lint", WIKI_LINT_SKILL),
    ("daily-synthesis-staging", DAILY_SYNTHESIS_STAGING_SKILL),
    ("daily-task-reminder", DAILY_TASK_REMINDER_SKILL),
    ("trading-chart", TRADING_CHART_SKILL),
];

pub(crate) fn ensure_builtin_skills(skills_root: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(skills_root)?;
    for (skill_name, content) in BUILTIN_SKILLS {
        let dir = skills_root.join(skill_name);
        std::fs::create_dir_all(&dir)?;
        let skill_file = dir.join("SKILL.md");
        if !skill_file.exists() {
            std::fs::write(skill_file, content)?;
        }
    }
    Ok(())
}

pub(crate) fn ensure_prompts_and_skills(
    prompts_dir: &Path,
    skills_dir: Option<&Path>,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(prompts_dir)?;
    for (filename, content) in [
        ("daily_root.md", DAILY_ROOT_PROMPT),
        ("wiki_researcher.md", WIKI_RESEARCHER_PROMPT),
        ("report_stager.md", REPORT_STAGER_PROMPT),
        ("media_creator.md", MEDIA_CREATOR_PROMPT),
        ("media_processor.md", MEDIA_PROCESSOR_PROMPT),
        ("reminder_planner.md", REMINDER_PLANNER_PROMPT),
        ("wiki_root.md", WIKI_ROOT_PROMPT),
        ("source_collector.md", SOURCE_COLLECTOR_PROMPT),
        ("wiki_ingestor.md", WIKI_INGESTOR_PROMPT),
        ("wiki_curator.md", WIKI_CURATOR_PROMPT),
    ] {
        let target = prompts_dir.join(filename);
        if !target.exists() {
            std::fs::write(target, content)?;
        }
    }

    if let Some(skills_root) = skills_dir {
        ensure_builtin_skills(skills_root)?;
    }
    Ok(())
}
