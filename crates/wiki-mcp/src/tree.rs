use wiki_core::TreeNode;

pub(super) fn render_tree(
    node: &TreeNode,
    prefix: &str,
    last: bool,
    out: &mut String,
    count: &mut usize,
) {
    let connector = if last { "└── " } else { "├── " };
    out.push_str(prefix);
    out.push_str(connector);
    out.push_str(&node.name);
    if node.is_directory {
        out.push('/');
    } else {
        *count += 1;
    }
    out.push('\n');
    let next = format!("{prefix}{}", if last { "    " } else { "│   " });
    for (index, child) in node.children.iter().enumerate() {
        render_tree(child, &next, index + 1 == node.children.len(), out, count);
    }
}

pub(super) fn limit_tree_depth(node: &mut TreeNode, depth: usize) {
    if depth == 0 {
        node.children.clear();
        return;
    }
    for child in &mut node.children {
        limit_tree_depth(child, depth - 1);
    }
}
