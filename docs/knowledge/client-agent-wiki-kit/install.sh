#!/usr/bin/env bash
# Cài kit vào một thư mục làm việc của client agent.
# Dùng: ./install.sh <thư-mục-đích> [codex|opencode|claude|all]
set -euo pipefail

KIT="$(cd "$(dirname "$0")" && pwd)"
DEST="${1:?Thiếu thư mục đích}"
CLIENT="${2:-all}"
mkdir -p "$DEST"

# Prompt hệ thống -> AGENTS.md (chèn nếu chưa có marker, không ghi đè nội dung cũ)
MARK="<!-- wiki-maintainer-kit -->"
if ! grep -qs "$MARK" "$DEST/AGENTS.md" 2>/dev/null; then
  { echo "$MARK"; sed -n '/^---$/,$p' "$KIT/prompts/00-system-wiki-maintainer.md" | tail -n +2; echo; } >> "$DEST/AGENTS.md"
  echo "AGENTS.md: đã thêm prompt hệ thống"
fi

copy_skills() {  # $1 = thư mục skills đích
  mkdir -p "$1"
  for d in "$KIT"/skills/*/; do
    mkdir -p "$1/$(basename "$d")"
    cp "$d/SKILL.md" "$1/$(basename "$d")/SKILL.md"
  done
  echo "skills -> $1"
}

mkdir -p "$DEST/wiki-kit" && cp -R "$KIT/prompts" "$KIT/skills" "$DEST/wiki-kit/"   # bản đọc được cho client không có skill loader
case "$CLIENT" in codex|all)    copy_skills "$DEST/.codex/skills" ;; esac
case "$CLIENT" in opencode|all) copy_skills "$DEST/.opencode/skills" ;; esac
case "$CLIENT" in claude|all)   copy_skills "$DEST/.claude/skills" ;; esac

echo "Xong. Còn lại: cấu hình MCP theo clients/ và export WIKI_MCP_TOKEN."
