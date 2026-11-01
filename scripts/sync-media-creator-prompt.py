#!/usr/bin/env python3
"""Đồng bộ mục video của prompt `media_creator` đã lưu với seed mới, giữ nguyên các mục khác.

`data/orchestrator-prompts/media_creator.md` được ghi đè từ `agent_settings.json` mỗi lần boot,
nên phải sửa file JSON. Dừng server trước khi chạy với --apply; bản gốc được lưu cạnh file.

    scripts/sync-media-creator-prompt.py <agent_settings.json> <prompts.rs> [--apply]
"""
import json
import re
import shutil
import sys
import time

VIDEO_SECTION = re.compile(r"^## \d+\. [^\n]*Video[^\n]*\n.*?(?=^## \d+\. |\Z)", re.M | re.S)
SEED = re.compile(r'MEDIA_CREATOR_PROMPT: &str = r#"(.*?)"#', re.S)


def video_section(prompt: str, source: str) -> re.Match:
    match = VIDEO_SECTION.search(prompt)
    if not match:
        sys.exit(f"{source}: không tìm thấy mục video")
    return match


def main() -> None:
    args = [arg for arg in sys.argv[1:] if arg != "--apply"]
    if len(args) != 2:
        sys.exit(__doc__)
    settings_path, seed_path = args
    apply = "--apply" in sys.argv[1:]

    seed = SEED.search(open(seed_path, encoding="utf-8").read())
    if not seed:
        sys.exit(f"{seed_path}: không tìm thấy MEDIA_CREATOR_PROMPT")
    seed_section = video_section(seed.group(1), seed_path).group(0).rstrip() + "\n\n"

    settings = json.load(open(settings_path, encoding="utf-8"))
    agent = settings.get("agents", {}).get("media_creator", {})
    prompt = agent.get("system_prompt")
    if not prompt or not prompt.strip():
        print("media_creator không có override: đã dùng seed mới, không cần đổi.")
        return

    current = video_section(prompt, settings_path)
    number = re.match(r"## (\d+)\.", current.group(0)).group(1)
    section = re.sub(r"^## \d+\.", f"## {number}.", seed_section, count=1)
    updated = prompt[: current.start()] + section + prompt[current.end() :]
    if updated == prompt:
        print("Mục video đã trùng seed.")
        return

    print(f"Thay {len(current.group(0))} ký tự mục video bằng {len(section)} ký tự từ seed.")
    if not apply:
        print("Dry-run: chưa ghi. Thêm --apply để ghi.")
        return
    backup = f"{settings_path}.{int(time.time())}.bak"
    shutil.copy2(settings_path, backup)
    agent["system_prompt"] = updated
    with open(settings_path, "w", encoding="utf-8") as output:
        json.dump(settings, output, ensure_ascii=False, indent=2)
    print(f"Đã ghi. Bản gốc: {backup}")


if __name__ == "__main__":
    main()
