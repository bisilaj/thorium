//! Result search tools for the Thorium MCP server.
//!
//! Provides full-text search across tool results and tags stored in
//! Elasticsearch. Agents use this to find relevant samples across the
//! entire corpus rather than browsing one sample at a time.

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use tracing::instrument;

use crate::models::ElasticSearchOpts;

use super::ThoriumMCP;

/// Default maximum number of search results to return.
const DEFAULT_MAX_RESULTS: usize = 20;

/// Hard ceiling on search results to prevent excessive Elasticsearch load.
const MAX_SEARCH_RESULTS: usize = 100;

/// Maximum character length for the excerpt field in search hits.
const MAX_EXCERPT_CHARS: usize = 500;

/// Maximum character length for a search query.
const MAX_QUERY_LENGTH: usize = 1000;

/// The params needed to search across tool results.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchResults {
    /// The search query using Lucene syntax (e.g. "mimikatz AND pe32",
    /// "PEType:PE32+"). Supports boolean operators (AND, OR), exact phrase
    /// matching with quotes, and field-specific queries.
    pub query: String,
    /// The groups to search within. At least one group must be specified.
    pub groups: Vec<String>,
    /// Maximum number of results to return (default: 20, max: 100).
    #[serde(default)]
    pub max_results: Option<usize>,
}

/// A projected search hit suitable for agent consumption.
///
/// Extracts identifying information from the Elasticsearch document source
/// rather than returning the raw document. For sample results, the `sha256`
/// field is populated. For repo results, the `url` field is populated
/// instead. Includes a bounded excerpt to help the agent decide which
/// results to explore further.
#[derive(Debug, Serialize, Deserialize)]
struct SearchHit {
    /// The SHA256 of the matching sample, if this is a sample result.
    sha256: Option<String>,
    /// The URL of the matching repo, if this is a repo result.
    url: Option<String>,
    /// The group this result belongs to.
    group: Option<String>,
    /// The Elasticsearch index this hit came from (e.g. sample results or tags).
    index: String,
    /// The relevance score for this hit, if available.
    score: Option<f64>,
    /// A bounded excerpt from the matching document to help the agent
    /// decide whether to explore this result further.
    excerpt: String,
}

/// Extract a bounded excerpt from an Elasticsearch document.
///
/// Prefers the `highlight` field (which contains search-relevant snippets)
/// over the raw `source`. Truncates to [`MAX_EXCERPT_CHARS`] to prevent
/// context bloat.
fn build_excerpt(
    source: &Option<serde_json::Value>,
    highlight: &Option<serde_json::Value>,
) -> String {
    // prefer highlight snippets if available
    if let Some(hl) = highlight {
        let text = serde_json::to_string(hl).unwrap_or_default();
        return truncate_string(&text, MAX_EXCERPT_CHARS);
    }
    // fall back to a preview of the source
    if let Some(src) = source {
        // extract the "results" field for a more focused excerpt
        let preview_value = src.get("results").unwrap_or(src);
        let text = serde_json::to_string(preview_value).unwrap_or_default();
        return truncate_string(&text, MAX_EXCERPT_CHARS);
    }
    String::new()
}

/// Truncate a string to a maximum number of characters, appending "..." if
/// truncation occurred.
fn truncate_string(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        text.to_owned()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

#[tool_router(router = search_router, vis = "pub")]
impl ThoriumMCP {
    /// Search across tool results and tags in Thorium using Elasticsearch.
    ///
    /// Uses Lucene query syntax for full-text search across all indexed
    /// tool results. Results are indexed per-group, so at least one group
    /// must be specified. Returns sample SHA256s (or repo URLs) and excerpts
    /// that can be used with `get_sample` and `get_sample_results` to
    /// explore matches.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "search_results",
        description = "Search across tool results in Thorium using Lucene query syntax. Requires at least one group (use list_groups to discover available groups). Returns matching sample SHA256 hashes with excerpts. Use get_sample and get_sample_results to explore the matches. Supports boolean operators (AND, OR), exact phrase matching with quotes, and field-specific queries."
    )]
    #[instrument(name = "ThoriumMCP::search_results", skip(self, parts), err(Debug))]
    pub async fn search_results(
        &self,
        Parameters(SearchResults {
            query,
            groups,
            max_results,
        }): Parameters<SearchResults>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate that the query is not empty
        if query.trim().is_empty() {
            return Err(ErrorData {
                code: rmcp::model::ErrorCode::INVALID_PARAMS,
                message: "Search query must not be empty".into(),
                data: None,
            });
        }
        // validate query length to prevent oversized Elasticsearch requests
        if query.len() > MAX_QUERY_LENGTH {
            return Err(ErrorData {
                code: rmcp::model::ErrorCode::INVALID_PARAMS,
                message: format!(
                    "Search query too long ({} chars, max {})",
                    query.len(),
                    MAX_QUERY_LENGTH
                )
                .into(),
                data: None,
            });
        }
        // validate that at least one group is specified
        if groups.is_empty() {
            return Err(ErrorData {
                code: rmcp::model::ErrorCode::INVALID_PARAMS,
                message: "At least one group must be specified for search".into(),
                data: None,
            });
        }
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // determine the max results, capped at the hard ceiling and at least 1
        let limit = max_results
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .min(MAX_SEARCH_RESULTS)
            .max(1);
        // build the search options
        let mut opts = ElasticSearchOpts::new(&query);
        opts.groups = groups;
        opts.limit = Some(limit);
        opts.page_size = limit;
        // execute the search
        let cursor = thorium.search.search(&opts).await?;
        // project each hit, extracting identifying fields from the ES document
        let hits: Vec<SearchHit> = cursor
            .data
            .iter()
            .map(|doc| {
                let source = doc.source.as_ref();
                // extract sha256 (sample results) or url (repo results)
                let sha256 = source
                    .and_then(|s| s.get("sha256"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let url = source
                    .and_then(|s| s.get("url"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let group = source
                    .and_then(|s| s.get("group"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
                // build a bounded excerpt
                let excerpt = build_excerpt(&doc.source, &doc.highlight);
                SearchHit {
                    sha256,
                    url,
                    group,
                    index: doc.index.clone(),
                    score: doc.score,
                    excerpt,
                }
            })
            .collect();
        // build the response
        let response = json!({
            "query": query,
            "hit_count": hits.len(),
            "hits": hits,
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
