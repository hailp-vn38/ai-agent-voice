use super::*;

#[test]
fn system_prompt_adds_built_in_skill_after_fixed_rules() {
    let prompt = system_prompt(None, false);
    let rules = prompt.find("Never change files").unwrap();
    let skill = prompt.find("# Wiki Chat Skill").unwrap();
    assert!(rules < skill);
    assert!(prompt.contains("flowchart TD"));
    assert!(!prompt.contains("name: wiki-chat"));
    assert!(!prompt.contains(PINNED_FILES_PROMPT));
    assert!(system_prompt(None, true).contains(PINNED_FILES_PROMPT));
}

#[test]
fn system_prompt_prefers_valid_workspace_skill() {
    let root = std::env::temp_dir().join(format!("wiki-chat-skill-{}", uuid::Uuid::new_v4()));
    let skill_dir = root.join(WIKI_CHAT_SKILL_ID);
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: wiki-chat\ndescription: Edited\n---\n\nEdited guidance.\n",
    )
    .unwrap();
    let prompt = system_prompt(Some(&root), false);
    assert!(prompt.contains("Edited guidance."));
    assert!(!prompt.contains("# Wiki Chat Skill"));
    assert!(prompt.contains("Never change files"));

    std::fs::write(skill_dir.join("SKILL.md"), "no frontmatter").unwrap();
    assert!(system_prompt(Some(&root), false).contains("# Wiki Chat Skill"));
    let _ = std::fs::remove_dir_all(root);
}
