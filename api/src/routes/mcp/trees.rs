//! Tree traversal tools for the Thorium MCP server.
//!
//! Trees discover relationships between samples, repos, entities, and tags
//! by crawling the Thorium data graph. An agent uses `start_tree` to find
//! related data starting from known samples, repos, or tags.
//!
//! The response is projected to lightweight node summaries rather than full
//! data objects. Use `get_sample`, `get_image`, or other detail tools to
//! fetch full information about specific nodes of interest.

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use tracing::instrument;
use uuid::Uuid;

use crate::models::{TreeNode, TreeOpts, TreeQuery, TreeRelatedQuery, TreeRelationships};

use super::ThoriumMCP;
use super::files::validate_sha256;

/// Default maximum number of nodes to return in a tree response.
const DEFAULT_NODE_LIMIT: usize = 25;

/// Hard ceiling on tree nodes to prevent context bloat.
const MAX_NODE_LIMIT: usize = 100;

/// The params needed to start a new tree.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StartTree {
    /// The SHA256 hashes of samples to start growing a tree of related
    /// data from. At least one of samples, repos, or tags must be provided.
    #[serde(default)]
    pub samples: Vec<String>,
    /// The repo URLs (e.g. "https://github.com/org/repo") to build this
    /// tree from.
    #[serde(default)]
    pub repos: Vec<String>,
    /// The entity UUIDs to build this tree from.
    #[serde(default)]
    pub entities: Vec<Uuid>,
    /// Tag filters to build this tree from. Each entry is a map of tag
    /// key to values (e.g. [{"malware_family": ["emotet", "trickbot"]}]).
    #[serde(default)]
    pub tags: Vec<BTreeMap<String, BTreeSet<String>>>,
    /// Optional group names to limit the tree search to. If empty, all
    /// accessible groups are searched.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Maximum number of nodes to return (default: 25, max: 100).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A projected summary of a tree node suitable for agent consumption.
///
/// Contains only the identifying information needed to understand the
/// node and decide whether to fetch full details.
#[derive(Debug, Serialize, Deserialize)]
struct TreeNodeSummary {
    /// The internal hash ID of this node in the tree.
    node_id: u64,
    /// The type of this node: "sample", "repo", "entity", or "tag".
    node_type: String,
    /// The primary identifier for this node (SHA256 for samples, URL for
    /// repos, UUID for entities, tag string for tags).
    identifier: String,
    /// A short label for this node (filename for samples, repo name for
    /// repos, entity name for entities, tag description for tags).
    label: String,
}

/// A projected summary of a tree branch (relationship between nodes).
#[derive(Debug, Serialize, Deserialize)]
struct TreeBranchSummary {
    /// The source node hash ID.
    from_node: u64,
    /// The target node hash ID.
    to_node: u64,
    /// A description of the relationship type.
    relationship: String,
}

/// Format a tree relationship as a clean, agent-readable string.
///
/// Produces concise labels like "Origin: Unpacked", "Association: FileFor"
/// instead of raw Rust Debug output.
fn describe_relationship(rel: &TreeRelationships) -> String {
    match rel {
        TreeRelationships::Initial => "Initial".to_owned(),
        TreeRelationships::Tags => "Tags".to_owned(),
        TreeRelationships::Origin(origin) => {
            let variant = match origin {
                crate::models::Origin::Downloaded { .. } => "Downloaded",
                crate::models::Origin::Unpacked { .. } => "Unpacked",
                crate::models::Origin::Transformed { .. } => "Transformed",
                crate::models::Origin::Wire { .. } => "Wire",
                crate::models::Origin::Incident { .. } => "Incident",
                crate::models::Origin::MemoryDump { .. } => "MemoryDump",
                crate::models::Origin::Source { .. } => "Source",
                crate::models::Origin::Carved { .. } => "Carved",
                crate::models::Origin::None => "None",
            };
            format!("Origin: {variant}")
        }
        TreeRelationships::Association(assoc) => {
            format!("Association: {}", assoc.kind)
        }
    }
}

/// Project a TreeNode into a lightweight summary.
fn project_tree_node(node_id: u64, node: &TreeNode) -> TreeNodeSummary {
    match node {
        TreeNode::Sample(sample) => {
            let label = sample
                .submissions
                .first()
                .and_then(|s| s.name.clone())
                .unwrap_or_else(|| sample.sha256[..16].to_owned());
            TreeNodeSummary {
                node_id,
                node_type: "sample".to_owned(),
                identifier: sample.sha256.clone(),
                label,
            }
        }
        TreeNode::Repo(repo) => TreeNodeSummary {
            node_id,
            node_type: "repo".to_owned(),
            identifier: repo.url.clone(),
            label: format!("{}/{}", repo.user, repo.name),
        },
        TreeNode::Entity(entity) => TreeNodeSummary {
            node_id,
            node_type: "entity".to_owned(),
            identifier: entity.id.to_string(),
            label: format!("{} ({})", entity.name, entity.kind.as_str()),
        },
        TreeNode::Tag(tags) => {
            let tag_str: String = tags
                .tags
                .iter()
                .map(|(k, vs)| {
                    let vals: Vec<&str> = vs.iter().map(|s| s.as_str()).collect();
                    format!("{}={}", k, vals.join(","))
                })
                .collect::<Vec<_>>()
                .join("; ");
            TreeNodeSummary {
                node_id,
                node_type: "tag".to_owned(),
                identifier: tag_str.clone(),
                label: tag_str,
            }
        }
    }
}

#[tool_router(router = tree_router, vis = "pub")]
impl ThoriumMCP {
    /// Find samples, repos, and entities related to a starting set by
    /// traversing relationships in the Thorium data graph.
    ///
    /// Returns projected node summaries (identifiers and types) and their
    /// relationships — not full data objects. Use `get_sample` or other
    /// detail tools to fetch full information about nodes of interest.
    ///
    /// At least one of `samples`, `repos`, `entities`, or `tags` must be
    /// provided as a starting point for the tree traversal.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "start_tree",
        description = "Find related samples, repos, and entities by traversing relationships (parent/child, tags, associations) from a starting set. Requires at least one sample SHA256, repo URL, entity UUID, or tag filter. Returns projected node summaries with relationship edges - use get_sample to explore specific nodes. For text-based discovery, use search_results instead."
    )]
    #[instrument(name = "ThoriumMCP::start_tree", skip(self, parts), err(Debug))]
    pub async fn start_tree(
        &self,
        Parameters(StartTree {
            samples,
            repos,
            entities,
            tags,
            groups,
            limit,
        }): Parameters<StartTree>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate that at least one input is provided
        if samples.is_empty() && repos.is_empty() && entities.is_empty() && tags.is_empty() {
            return Err(ErrorData {
                code: rmcp::model::ErrorCode::INVALID_PARAMS,
                message: "At least one of samples, repos, entities, or tags must be provided"
                    .into(),
                data: None,
            });
        }
        // validate sha256 format for all samples
        for sample in &samples {
            validate_sha256(sample)?;
        }
        // determine the node limit
        let node_limit = limit
            .unwrap_or(DEFAULT_NODE_LIMIT)
            .min(MAX_NODE_LIMIT)
            .max(1);
        // build the query for starting a new tree
        let query = TreeQuery {
            groups,
            samples,
            repos,
            entities,
            tags,
            related: TreeRelatedQuery::default(),
        };
        // use default traversal opts (depth=50); the node_limit only caps
        // how many nodes we return, not how deep the tree grows
        let opts = TreeOpts::default();
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // grow a tree based on our initial query
        let tree = thorium.trees.start(&opts, &query).await?;
        // collect initial node IDs for prioritization
        let initial_ids: std::collections::HashSet<u64> =
            tree.initial.iter().copied().collect();
        // sort data_map entries deterministically: initial nodes first, then by id
        let mut sorted_entries: Vec<(&u64, &TreeNode)> = tree.data_map.iter().collect();
        sorted_entries.sort_by_key(|(id, _)| (!initial_ids.contains(id), **id));
        // project tree nodes to lightweight summaries, capped at the limit
        let nodes: Vec<TreeNodeSummary> = sorted_entries
            .into_iter()
            .take(node_limit)
            .map(|(&id, node)| project_tree_node(id, node))
            .collect();
        // collect the node IDs we included for filtering branches
        let included_ids: std::collections::HashSet<u64> =
            nodes.iter().map(|n| n.node_id).collect();
        // project branches to relationship summaries, only for included nodes
        let mut edges: Vec<TreeBranchSummary> = Vec::new();
        for (&from_id, branches) in &tree.branches {
            for branch in branches {
                // only include edges between nodes in our result set
                if included_ids.contains(&from_id) && included_ids.contains(&branch.node) {
                    edges.push(TreeBranchSummary {
                        from_node: from_id,
                        to_node: branch.node,
                        relationship: describe_relationship(&branch.relationship),
                    });
                }
            }
        }
        edges.sort_by_key(|e| (e.from_node, e.to_node));
        // check if we truncated the node set
        let total_nodes = tree.data_map.len();
        let truncated = total_nodes > node_limit;
        // identify which nodes can be further explored
        let growable: Vec<u64> = tree
            .growable
            .iter()
            .filter(|id| included_ids.contains(id))
            .copied()
            .collect();
        // build the response
        let response = json!({
            "tree_id": tree.id.to_string(),
            "node_count": nodes.len(),
            "total_nodes_in_tree": total_nodes,
            "truncated": truncated,
            "edge_count": edges.len(),
            "growable_count": growable.len(),
            "nodes": nodes,
            "edges": edges,
        });
        let serialized = serde_json::to_value(&response).unwrap();
        // build our result
        let result = CallToolResult {
            content: vec![Content::json(&response)?],
            structured_content: Some(serialized),
            is_error: Some(false),
            meta: None,
        };
        Ok(result)
    }
}
