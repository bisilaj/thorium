//! Sample and result tools for the Thorium MCP server.
//!
//! These tools give AI agents access to samples (files) and their analysis
//! results. Result access follows a two-step pattern:
//!
//! 1. [`get_sample_results`](ThoriumMCP::get_sample_results) returns a summary
//!    of which tools have produced results for a sample, without including the
//!    actual result content.
//! 2. [`get_sample_result`](ThoriumMCP::get_sample_result) fetches the full
//!    result content for a specific tool, with optional truncation.

use std::collections::HashMap;

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use tracing::instrument;

use crate::client::ResultsClient;
use crate::models::ResultGetParams;

use super::ThoriumMCP;

/// Validate that a string looks like a SHA256 hash (64 hex characters).
fn validate_sha256(sha256: &str) -> Result<(), ErrorData> {
    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ErrorData {
            code: rmcp::model::ErrorCode::INVALID_PARAMS,
            message: "Invalid SHA256: must be exactly 64 hexadecimal characters".into(),
            data: None,
        });
    }
    Ok(())
}

/// The params needed to get info about a sample.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Sha256 {
    /// The SHA256 hash of the sample to get info on.
    pub sha256: String,
}

/// A projected view of a sample suitable for agent consumption.
///
/// Omits the full submissions array and comments to keep context compact.
/// Tags are flattened to key-value pairs with their group scopes removed
/// for readability.
#[derive(Debug, Serialize, Deserialize)]
struct SampleSummary {
    /// The SHA256 hash of this sample.
    sha256: String,
    /// The SHA1 hash of this sample.
    sha1: String,
    /// The MD5 hash of this sample.
    md5: String,
    /// The groups this sample is visible in.
    groups: Vec<String>,
    /// The tags for this sample as key -> values mappings.
    tags: std::collections::HashMap<String, Vec<String>>,
    /// When this sample was first uploaded (ISO 8601).
    uploaded: Option<String>,
    /// The original file name, if one was provided.
    name: Option<String>,
    /// A description of this sample, if one was provided.
    description: Option<String>,
    /// The number of submissions for this sample.
    submission_count: usize,
    /// The number of comments on this sample.
    comment_count: usize,
}

/// The params for listing which tools have produced results for a sample.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetSampleResults {
    /// The SHA256 hash of the sample to get results for.
    pub sha256: String,
    /// Limit results to specific tools (image names). If empty, all tools are included.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Limit results to specific groups. If empty, all accessible groups are included.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Whether to include hidden results (default: false).
    #[serde(default)]
    pub include_hidden: bool,
}

/// The params for fetching a specific tool's result content for a sample.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetSampleResult {
    /// The SHA256 hash of the sample to get the result for.
    pub sha256: String,
    /// The name of the tool (image) whose results to fetch.
    pub tool: String,
    /// Limit results to a specific group. If not set, all accessible groups are included.
    #[serde(default)]
    pub group: Option<String>,
    /// Maximum number of characters to return per result value. If the JSON
    /// serialization of a result exceeds this limit it will be truncated and
    /// the `truncated` flag will be set to true. If not set, the full result
    /// is returned.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// A summary of results for a single tool — no content included.
#[derive(Debug, Serialize, Deserialize)]
struct ToolResultSummary {
    /// The number of result entries for this tool.
    result_count: usize,
    /// How results from this tool are rendered (e.g. "Json", "String", "Table").
    display_type: String,
    /// When the most recent result was uploaded (ISO 8601).
    latest_uploaded: String,
    /// Whether any results have associated files.
    has_files: bool,
}

/// A single result entry with optional truncation.
#[derive(Debug, Serialize, Deserialize)]
struct ResultEntry {
    /// The unique ID of this result.
    id: String,
    /// The command that generated this result, if recorded.
    cmd: Option<String>,
    /// When this result was uploaded (ISO 8601).
    uploaded: String,
    /// How this result is rendered (e.g. "Json", "String", "Table").
    display_type: String,
    /// The actual result content.
    result: serde_json::Value,
    /// Any files associated with this result.
    files: Vec<String>,
    /// Whether the result content was truncated due to max_chars.
    truncated: bool,
}

/// Truncate a JSON value's string representation to a maximum number of
/// characters. Returns the original value if it fits, or a string value
/// containing the truncated representation.
fn truncate_json_value(value: &serde_json::Value, max_chars: usize) -> (serde_json::Value, bool) {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    let char_count = serialized.chars().count();
    if char_count <= max_chars {
        (value.clone(), false)
    } else {
        // truncate and append an indicator
        let truncated: String = serialized.chars().take(max_chars).collect();
        let message = format!("{}... [truncated, {} total chars]", truncated, char_count);
        (serde_json::Value::String(message), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_json_value_returns_original_when_under_limit() {
        let value = json!({"key": "short"});
        let (result, truncated) = truncate_json_value(&value, 1000);
        assert_eq!(result, value);
        assert!(!truncated);
    }

    #[test]
    fn truncate_json_value_returns_original_at_exact_limit() {
        let value = json!("hello");
        // serde_json::to_string for "hello" is `"hello"` which is 7 chars
        let (result, truncated) = truncate_json_value(&value, 7);
        assert_eq!(result, value);
        assert!(!truncated);
    }

    #[test]
    fn truncate_json_value_truncates_when_over_limit() {
        let value = json!({"key": "a long value that should be truncated"});
        let (result, truncated) = truncate_json_value(&value, 10);
        assert!(truncated);
        // the result should be a string containing the truncation indicator
        let result_str = result.as_str().unwrap();
        assert!(result_str.contains("... [truncated,"));
        assert!(result_str.contains("total chars]"));
    }

    #[test]
    fn truncate_json_value_handles_empty_object() {
        let value = json!({});
        let (result, truncated) = truncate_json_value(&value, 1000);
        assert_eq!(result, value);
        assert!(!truncated);
    }

    #[test]
    fn truncate_json_value_handles_multibyte_chars() {
        // test that char counting works correctly with multi-byte UTF-8
        let value = json!("cafe\u{0301}"); // "cafe" + combining accent = "café"
        let serialized = serde_json::to_string(&value).unwrap();
        let char_count = serialized.chars().count();
        // truncate to exactly the char count should not truncate
        let (_, truncated) = truncate_json_value(&value, char_count);
        assert!(!truncated);
        // truncate to one less should truncate
        let (_, truncated) = truncate_json_value(&value, char_count - 1);
        assert!(truncated);
    }
}

#[tool_router(router = sample_router, vis = "pub")]
impl ThoriumMCP {
    /// Get basic info about a specific sample/file by SHA256.
    ///
    /// Returns projected metadata (hashes, groups, tags, upload date, name)
    /// rather than the full sample object. Use this to understand a sample
    /// before exploring its results.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_sample",
        description = "Get basic info about a specific sample/file by its SHA256 hash. Requires the sample's SHA256 hash - use search_results to find samples by content. Returns metadata such as hashes, groups, tags, upload date, and file name. Use get_sample_results to see what tools have been run on this sample."
    )]
    #[instrument(name = "ThoriumMCP::get_sample", skip(self, parts), err(Debug))]
    pub async fn get_sample(
        &self,
        Parameters(Sha256 { sha256 }): Parameters<Sha256>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate sha256 format
        validate_sha256(&sha256)?;
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // get this sample
        let sample = thorium.files.get(&sha256).await?;
        // collect deduplicated groups from all submissions
        let groups: Vec<String> = sample.groups().into_iter().map(String::from).collect();
        // flatten tags to key -> [values] without group scopes
        let tags: std::collections::HashMap<String, Vec<String>> = sample
            .tags
            .iter()
            .map(|(key, values_map)| {
                let values: Vec<String> = values_map.keys().cloned().collect();
                (key.clone(), values)
            })
            .collect();
        // get the earliest upload time and first submission details
        let first_submission = sample.submissions.first();
        let uploaded = first_submission.map(|s| s.uploaded.to_rfc3339());
        let name = first_submission.and_then(|s| s.name.clone());
        let description = first_submission.and_then(|s| s.description.clone());
        // project the sample
        let summary = SampleSummary {
            sha256: sample.sha256,
            sha1: sample.sha1,
            md5: sample.md5,
            groups,
            tags,
            uploaded,
            name,
            description,
            submission_count: sample.submissions.len(),
            comment_count: sample.comments.len(),
        };
        // serialize our projected sample
        let serialized = serde_json::to_value(&summary).unwrap();
        // build our result
        let result = CallToolResult {
            content: vec![Content::json(&summary)?],
            structured_content: Some(serialized),
            is_error: Some(false),
            meta: None,
        };
        Ok(result)
    }

    /// Get a summary of which tools have produced results for a sample.
    ///
    /// Returns a map of tool names to result summaries (count, display type,
    /// latest upload time, whether files are attached). Does NOT include the
    /// actual result content — use `get_sample_result` to fetch content for
    /// a specific tool.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_sample_results",
        description = "Get a summary of which tools have produced results for a sample. Returns tool names with result counts and metadata, but NOT the actual result content. Use get_sample_result to fetch content for a specific tool."
    )]
    #[instrument(name = "ThoriumMCP::get_sample_results", skip(self, parts), err(Debug))]
    pub async fn get_sample_results(
        &self,
        Parameters(GetSampleResults {
            sha256,
            tools,
            groups,
            include_hidden,
        }): Parameters<GetSampleResults>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate sha256 format
        validate_sha256(&sha256)?;
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // build our results get params with any filters
        let mut params = ResultGetParams::default();
        if include_hidden {
            params = params.hidden();
        }
        if !tools.is_empty() {
            params = params.tools(tools);
        }
        if !groups.is_empty() {
            params = params.groups(groups);
        }
        // get this sample's results
        let sample_results = thorium.files.get_results(&sha256, &params).await?;
        // build a summary map without the actual result content
        let mut tool_summaries = HashMap::with_capacity(sample_results.results.len());
        for (tool_name, results) in &sample_results.results {
            // find the most recently uploaded result
            let latest = results
                .iter()
                .map(|r| r.uploaded)
                .max()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            // check if any results have files
            let has_files = results.iter().any(|r| !r.files.is_empty());
            // get the display type from the first result
            let display_type = results
                .first()
                .map(|r| r.display_type.as_str())
                .unwrap_or("Json")
                .to_owned();
            tool_summaries.insert(
                tool_name.clone(),
                ToolResultSummary {
                    result_count: results.len(),
                    display_type,
                    latest_uploaded: latest,
                    has_files,
                },
            );
        }
        // build the response
        let response = json!({
            "sha256": sha256,
            "tool_count": tool_summaries.len(),
            "tools": tool_summaries,
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

    /// Get the full result content for a specific tool's analysis of a sample.
    ///
    /// Use `get_sample_results` first to discover which tools have run, then
    /// call this tool to fetch the actual content for a specific tool. The
    /// optional `max_chars` parameter truncates large results to keep context
    /// manageable.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_sample_result",
        description = "Get the full result content for a specific tool's analysis of a sample. Use get_sample_results first to discover which tools have run, then call this to fetch the actual content. Set max_chars to truncate large results."
    )]
    #[instrument(name = "ThoriumMCP::get_sample_result", skip(self, parts), err(Debug))]
    pub async fn get_sample_result(
        &self,
        Parameters(GetSampleResult {
            sha256,
            tool,
            group,
            max_chars,
        }): Parameters<GetSampleResult>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate sha256 format
        validate_sha256(&sha256)?;
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // build our results get params filtering to this specific tool
        let mut params = ResultGetParams::default().tool(&tool);
        if let Some(ref group) = group {
            params = params.group(group);
        }
        // get the results for this specific tool
        let sample_results = thorium.files.get_results(&sha256, &params).await?;
        // extract the results for our tool
        let tool_results = sample_results.results.get(&tool).cloned().unwrap_or_default();
        // build result entries with optional truncation
        let entries: Vec<ResultEntry> = tool_results
            .iter()
            .map(|output| {
                let (result_value, truncated) = match max_chars {
                    Some(max) => truncate_json_value(&output.result, max),
                    None => (output.result.clone(), false),
                };
                ResultEntry {
                    id: output.id.to_string(),
                    cmd: output.cmd.clone(),
                    uploaded: output.uploaded.to_rfc3339(),
                    display_type: output.display_type.as_str().to_owned(),
                    result: result_value,
                    files: output.files.clone(),
                    truncated,
                }
            })
            .collect();
        // build the response
        let response = json!({
            "sha256": sha256,
            "tool": tool,
            "result_count": entries.len(),
            "results": entries,
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
