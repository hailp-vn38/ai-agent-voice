"""Deterministic offline research fixture; does not fetch live websites."""
from __future__ import annotations

import json
import re
from pathlib import Path

FIXTURE_DATA = Path(__file__).with_name("fixture_posts.json")


def collect_topic(topic: str, limit: int) -> dict:
    posts = json.loads(FIXTURE_DATA.read_text(encoding="utf-8"))
    keyword = topic.casefold().strip()
    pattern = re.compile(r"(?<!\w)" + re.escape(keyword) + r"(?!\w)")
    matches = [
        post for post in posts
        if pattern.search((post["title"] + " " + post["text"]).casefold())
    ][:limit]
    count = len(matches)
    summary = (
        f"Đã tìm thấy {count} bài viết mẫu về '{topic}'. "
        "Dữ liệu hoàn toàn giả lập, không lấy từ mạng."
    )
    return {
        "source_id": "fixture",
        "topic": topic,
        "record_count": count,
        "summary": summary,
        "records": matches,
        "next_cursor": None,
        "done": True,
        "synthetic": True,
    }


def make_report(result: dict) -> str:
    lines = [
        f"# Báo cáo mẫu: {result['topic']}",
        "",
        "> Dữ liệu giả lập (fixture), không phải thông tin từ Weibo hoặc X.",
        "",
        result["summary"],
        "",
        "## Bài viết",
        "",
    ]
    for post in result["records"]:
        lines.extend([f"- **{post['title']}** (`{post['id']}`): {post['text']}"])
    if not result["records"]:
        lines.append("Không có bài viết phù hợp.")
    lines.append("")
    return "\n".join(lines)
