use super::*;

#[test]
fn headings_overlap_and_byte_offsets_cover_original_without_gaps() {
    let body = format!(
        "# Giới thiệu 😀\r\n{}\r\n## Tối ưu\r\n{}\r\n",
        "Thông tin tiếng Việt. ".repeat(240),
        "Câu hỏi Agent / Rust. ".repeat(250)
    );
    let chunks = split_markdown(&body);
    assert!(chunks.len() >= 3);
    let mut covered = 0usize;
    for (idx, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.ordinal, idx as i64);
        assert_eq!(&body[chunk.start_byte..chunk.end_byte], chunk.text);
        assert_eq!(chunk.sha256.len(), 64);
        assert!(chunk.start_byte <= covered, "gap in original Markdown");
        covered = covered.max(chunk.end_byte);
        assert!(chunk.start_line <= chunk.end_line);
    }
    assert_eq!(covered, body.len());
}

#[test]
fn a_long_fenced_block_is_not_sliced_inside() {
    for delimiter in ["~~~", "\u{60}\u{60}\u{60}\u{60}"] {
        let body = format!(
            "# Introduction\n{}\n{}rust\n{}\n{}\n# Last\nEnd\n",
            "Introductory prose. ".repeat(190),
            delimiter,
            "println!(\"hello\");\n".repeat(220),
            delimiter
        );
        let chunks = split_markdown(&body);
        assert!(chunks.len() >= 2);
        let fence_start = body.find(&format!("{delimiter}rust")).unwrap();
        let fence_end = body[fence_start + delimiter.len()..]
            .find(&format!("\n{delimiter}\n"))
            .map(|offset| fence_start + delimiter.len() + offset + 1 + delimiter.len())
            .unwrap();
        for chunk in chunks {
            assert!(
                chunk.start_byte <= fence_start || chunk.start_byte >= fence_end,
                "chunk starts inside fenced block"
            );
            assert!(
                chunk.end_byte <= fence_start || chunk.end_byte >= fence_end,
                "chunk ends inside fenced block"
            );
        }
    }
}

#[test]
fn empty_and_unicode_only_body() {
    assert!(split_markdown("").is_empty());
    let body = "😀🦀 tiếng Việt";
    let chunk = split_markdown(body);
    assert_eq!(chunk.len(), 1);
    assert_eq!(chunk[0].text, body);
    assert_eq!(chunk[0].start_line, 1);
    assert_eq!(chunk[0].end_line, 1);
}
