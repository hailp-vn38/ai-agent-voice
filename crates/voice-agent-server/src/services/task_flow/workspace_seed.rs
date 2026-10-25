//! Default agent system prompts and skills seeded into the workspace if absent.
//!
//! Canonical prompt and skill constants are shared from [`orchestrator_flow_rs::prompts`]
//! so both repositories maintain a single source of truth.

use std::path::Path;

pub use orchestrator_flow_rs::{
    DAILY_ROOT_PROMPT, DAILY_SYNTHESIS_STAGING_SKILL, DAILY_TASK_REMINDER_SKILL,
    MEDIA_CREATOR_PROMPT, REMINDER_PLANNER_PROMPT, REPORT_STAGER_PROMPT, SOURCE_COLLECTOR_PROMPT,
    WIKI_CURATOR_PROMPT, WIKI_INGESTOR_PROMPT, WIKI_RESEARCHER_PROMPT, WIKI_ROOT_PROMPT,
};

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
        std::fs::create_dir_all(skills_root)?;
        for (skill_name, content) in [
            ("daily-synthesis-staging", DAILY_SYNTHESIS_STAGING_SKILL),
            ("daily-task-reminder", DAILY_TASK_REMINDER_SKILL),
        ] {
            let dir = skills_root.join(skill_name);
            std::fs::create_dir_all(&dir)?;
            let skill_file = dir.join("SKILL.md");
            if !skill_file.exists() {
                std::fs::write(skill_file, content)?;
            }
        }
    }
    Ok(())
}
