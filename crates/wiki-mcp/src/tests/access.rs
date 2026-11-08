#[test]
fn read_write_access_lists_only_read_tools_when_policy_disables_writes() {
    let (_dir, host) = host(false);
    let tools = host.list_tools(Access::ReadWrite);
    assert!(tools.iter().any(|tool| tool.name == "wiki_status"));
    assert!(!tools.iter().any(|tool| tool.name == "wiki_write_file"));
}
use super::host;
use crate::Access;
