use super::*;

#[test]
fn web_assistant_sees_server_tool_status_even_with_mcp_disabled_and_no_opt_in() {
    let features = crate::protocol::ClientFeatures {
        mcp: false,
        tool_status: false,
        ..Default::default()
    };
    assert!(tool_status_enabled(true, &features));
    assert!(!tool_status_enabled(false, &features));

    let opted_in = crate::protocol::ClientFeatures {
        tool_status: true,
        mcp: false,
        ..Default::default()
    };
    assert!(tool_status_enabled(false, &opted_in));
    assert!(tool_status_enabled(true, &opted_in));
}
