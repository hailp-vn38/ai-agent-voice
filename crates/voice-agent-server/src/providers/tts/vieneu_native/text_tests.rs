use super::frame_limit;

#[test]
fn frame_caps_match_upstream_for_short_and_long_phrases() {
    assert_eq!(frame_limit("tʃˈaː2w."), 13);
    assert_eq!(frame_limit("sˈin tʃˈaː2w."), 18);
    assert_eq!(frame_limit("kwˈaːɜ"), 13);
    assert_eq!(frame_limit("kɹiːˈeɪt"), 18);
    assert_eq!(frame_limit("kəmpjˈuːɾɚ"), 23);
    assert_eq!(frame_limit(&"sˈin ".repeat(100)), 300);
}
