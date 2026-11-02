use llm_wiki::GraphData;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[allow(dead_code)] // Read and Cited are populated by the follow-up ask-loop ticket.
pub(super) enum EvidenceTier {
    Retrieved,
    Read,
    Cited,
}

impl EvidenceTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cited => "cited",
            Self::Read => "read",
            Self::Retrieved => "retrieved",
        }
    }
}

#[derive(Clone, Debug)]
struct EvidenceEntry {
    tier: EvidenceTier,
    score: Option<f32>,
}

#[derive(Default, Clone, Debug)]
pub(super) struct Evidence {
    entries: HashMap<String, EvidenceEntry>,
}

impl Evidence {
    pub(super) fn record(&mut self, rel_path: &str, tier: EvidenceTier, score: Option<f32>) {
        let entry = self
            .entries
            .entry(rel_path.to_owned())
            .or_insert(EvidenceEntry { tier, score });
        if tier > entry.tier {
            entry.tier = tier;
        }
        if let Some(score) = score {
            entry.score = Some(entry.score.map_or(score, |current| current.max(score)));
        }
    }
}

pub(super) struct AnswerGraphLimits {
    pub max_nodes: usize,
}

pub(super) fn answer_graph(
    data: GraphData,
    evidence: &Evidence,
    limits: AnswerGraphLimits,
) -> Value {
    let allowed_nodes: HashMap<_, _> = data
        .nodes
        .into_iter()
        .filter(|node| {
            node.category.as_deref() != Some("system")
                && node.rel_path != "wiki/index.md"
                && node.rel_path != "wiki/log.md"
        })
        .map(|node| (node.rel_path.clone(), node))
        .collect();

    let mut seeds: Vec<_> = evidence
        .entries
        .iter()
        .filter(|(path, _)| allowed_nodes.contains_key(*path))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect();
    seeds.sort_by(|(left_path, left), (right_path, right)| {
        right
            .tier
            .cmp(&left.tier)
            .then_with(|| {
                right
                    .score
                    .unwrap_or_default()
                    .total_cmp(&left.score.unwrap_or_default())
            })
            .then_with(|| left_path.cmp(right_path))
    });
    let seed_ids: HashSet<_> = seeds.iter().map(|(path, _)| path.clone()).collect();
    let mut neighbor_seeds: HashMap<String, HashSet<String>> = HashMap::new();
    for edge in &data.edges {
        let Some(target) = edge.resolved_rel_path.as_deref() else {
            continue;
        };
        if edge.source_rel_path == target
            || !allowed_nodes.contains_key(&edge.source_rel_path)
            || !allowed_nodes.contains_key(target)
        {
            continue;
        }
        if seed_ids.contains(&edge.source_rel_path) && !seed_ids.contains(target) {
            neighbor_seeds
                .entry(target.to_owned())
                .or_default()
                .insert(edge.source_rel_path.clone());
        }
        if seed_ids.contains(target) && !seed_ids.contains(&edge.source_rel_path) {
            neighbor_seeds
                .entry(edge.source_rel_path.clone())
                .or_default()
                .insert(target.to_owned());
        }
    }

    let mut contexts: Vec<_> = neighbor_seeds
        .into_iter()
        .map(|(path, neighbors)| (path, neighbors.len()))
        .collect();
    contexts.sort_by(|(left_path, left_count), (right_path, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_path.cmp(right_path))
    });

    // Evidence is never dropped to make room for context; the cap applies to
    // context nodes and reports a partial graph when evidence fills the cap.
    let truncated = seeds.len().saturating_add(contexts.len()) > limits.max_nodes;
    let remaining = limits.max_nodes.saturating_sub(seeds.len());
    contexts.truncate(remaining);
    let mut ordered_paths: Vec<_> = seeds.iter().map(|(path, _)| path.clone()).collect();
    ordered_paths.extend(contexts.iter().map(|(path, _)| path.clone()));
    let selected_context: HashSet<_> = contexts.into_iter().map(|(path, _)| path).collect();
    let mut included = seed_ids.clone();
    included.extend(selected_context);

    let nodes: Vec<_> = ordered_paths
        .iter()
        .filter_map(|path| {
            let node = allowed_nodes.get(path)?;
            let entry = evidence.entries.get(path);
            Some(json!({
                "id": node.rel_path,
                "path": node.rel_path,
                "title": node.title,
                "status": "verified",
                "revision": 1,
                "docid": node.docid,
                "category": node.category,
                "tags": node.tags,
                "tier": entry.map(|e| e.tier.as_str()).unwrap_or("context"),
                "score": entry.and_then(|e| e.score),
            }))
        })
        .collect();
    let edges = aggregate_graph_edges(&data.edges, &included);

    json!({
        "nodes": nodes,
        "edges": edges,
        "unresolved": [],
        "unresolved_targets": [],
        "truncated": truncated,
    })
}

pub(super) fn aggregate_graph_edges(
    edges: &[llm_wiki::GraphEdge],
    kept_node_ids: &HashSet<String>,
) -> Vec<Value> {
    let mut aggregated: Vec<AggregatedGraphEdge> = Vec::new();
    let mut edge_index: HashMap<(String, String, String), usize> = HashMap::new();

    for edge in edges {
        let Some(target_id) = edge.resolved_rel_path.as_ref() else {
            continue;
        };
        if edge.source_rel_path == *target_id
            || !kept_node_ids.contains(&edge.source_rel_path)
            || !kept_node_ids.contains(target_id)
        {
            continue;
        }
        let reference = json!({
            "line": edge.line_number,
            "raw_target": edge.raw_target,
            "alias": edge.display_alias,
            "anchor": edge.target_heading,
        });
        let key = (
            edge.source_rel_path.clone(),
            target_id.clone(),
            edge.link_type.clone(),
        );
        if let Some(&index) = edge_index.get(&key) {
            let entry = &mut aggregated[index];
            if !entry.lines.contains(&edge.line_number) {
                entry.lines.push(edge.line_number);
            }
            entry.references.push(reference);
        } else {
            edge_index.insert(key, aggregated.len());
            aggregated.push(AggregatedGraphEdge {
                source_id: edge.source_rel_path.clone(),
                target_id: target_id.clone(),
                kind: edge.link_type.clone(),
                lines: vec![edge.line_number],
                references: vec![reference],
            });
        }
    }

    aggregated
        .into_iter()
        .map(|edge| {
            json!({
                "source_id": edge.source_id,
                "target_id": edge.target_id,
                "kind": edge.kind,
                "lines": edge.lines,
                "references": edge.references,
            })
        })
        .collect()
}

struct AggregatedGraphEdge {
    source_id: String,
    target_id: String,
    kind: String,
    lines: Vec<usize>,
    references: Vec<Value>,
}
