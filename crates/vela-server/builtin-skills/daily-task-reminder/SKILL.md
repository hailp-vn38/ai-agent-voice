---
name: daily-task-reminder
description: "Workflow for extracting actionable tasks, deadlines, recurring schedules, and linked Wiki references to create/manage user reminders."
---

# Daily Task & Reminder Planning Skill (`daily-task-reminder`)

Use this skill when creating, listing, or updating reminders and action items for the user.

## Rules
1. **Check Existing Reminders First**: Before creating a new reminder, call `reminder_list(status="pending")` to avoid creating duplicate reminders for the same task.
2. **Normalize Time & Recurrence**:
   - Resolve relative time expressions into strict ISO-8601 format (`YYYY-MM-DDTHH:MM:SS+07:00`).
3. **Attach Wiki Context (`related_wiki_links`)**:
   - If the reminder relates to a topic in the Wiki, include the exact `[[wiki/sources/...]]`, `[[concepts/...]]`, or `[[syntheses/...]]` links in `related_wiki_links`.
