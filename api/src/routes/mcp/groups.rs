//! Group discovery tools for the Thorium MCP server.
//!
//! Groups are the multi-tenancy boundary in Thorium. Nearly every operation
//! requires a group name, so discovering available groups is typically the
//! first step in an agent workflow.

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use serde_json::json;
use tracing::instrument;

use super::ThoriumMCP;

/// A projected view of a group suitable for agent consumption.
///
/// Only the fields an agent needs for discovery are included; membership
/// lists, allowed-data rules, and other administrative details are omitted.
#[derive(Debug, Serialize, Deserialize)]
struct GroupSummary {
    /// The name of this group.
    name: String,
    /// A human-readable description of the group, if one was set.
    description: Option<String>,
}

#[tool_router(router = groups_router, vis = "pub")]
impl ThoriumMCP {
    /// List the groups accessible to the current user.
    ///
    /// Groups are the multi-tenancy boundary in Thorium — nearly every
    /// operation requires a group name. Call this first to discover which
    /// groups are available before listing pipelines, images, or samples.
    ///
    /// # Errors
    ///
    /// Returns an authentication error if the MCP session token is invalid
    /// or the Thorium API is unreachable.
    #[tool(
        name = "list_groups",
        description = "List the groups accessible to the current user. Groups are the multi-tenancy boundary in Thorium - nearly every operation requires a group name. Call this first to discover which groups are available."
    )]
    #[instrument(name = "ThoriumMCP::list_groups", skip(self, parts), err(Debug))]
    pub async fn list_groups(
        &self,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client (also validates the auth token)
        let thorium = self.conf.client(&parts).await?;
        // list groups with details so we get descriptions
        let mut cursor = thorium.groups.list().details().limit(500);
        // fetch the first page of groups
        cursor.next().await?;
        // project each group down to the fields an agent needs
        let summaries: Vec<GroupSummary> = cursor
            .details
            .iter()
            .map(|group| GroupSummary {
                name: group.name.clone(),
                description: group.description.clone(),
            })
            .collect();
        // check if there are more groups beyond this page
        let has_more = !cursor.exhausted;
        // serialize our projected groups
        let response = json!({
            "groups": &summaries,
            "has_more": has_more,
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
