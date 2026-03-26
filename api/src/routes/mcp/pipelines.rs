//! Pipeline discovery tools for the Thorium MCP server.
//!
//! Pipelines define reusable blueprints of ordered image stages. An agent
//! uses these tools to understand what analysis workflows are available
//! before creating reactions.

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use tracing::instrument;

use super::ThoriumMCP;

/// Default maximum number of pipelines to return from a list operation.
const DEFAULT_LIST_LIMIT: u64 = 100;

/// The params needed to list pipelines in a group.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListPipelines {
    /// The name of the group to list pipelines for.
    pub group: String,
    /// Maximum number of pipelines to return (default: 100).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// The params needed to get details about a specific pipeline.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetPipeline {
    /// The name of the group this pipeline belongs to.
    pub group: String,
    /// The name of the pipeline to get details for.
    pub pipeline: String,
}

/// A projected view of a pipeline suitable for list results.
///
/// Only the fields an agent needs for discovery are included; triggers,
/// bans, and other administrative details are omitted.
#[derive(Debug, Serialize, Deserialize)]
struct PipelineSummary {
    /// The name of this pipeline.
    name: String,
    /// The group this pipeline belongs to.
    group: String,
    /// A human-readable description of the pipeline, if one was set.
    description: Option<String>,
    /// The number of stages in this pipeline.
    stage_count: usize,
    /// The default SLA for reactions in seconds.
    sla: u64,
    /// The number of auto-triggers configured for this pipeline.
    trigger_count: usize,
}

/// A detailed view of a pipeline for understanding its structure.
#[derive(Debug, Serialize, Deserialize)]
struct PipelineDetail {
    /// The name of this pipeline.
    name: String,
    /// The group this pipeline belongs to.
    group: String,
    /// A human-readable description of the pipeline, if one was set.
    description: Option<String>,
    /// The ordered stages and their images.
    ///
    /// Each inner list represents images that run in parallel within a stage.
    /// Stages execute sequentially.
    order: Vec<Vec<String>>,
    /// The default SLA for reactions in seconds.
    sla: u64,
    /// The total number of images across all stages.
    image_count: usize,
}

#[tool_router(router = pipelines_router, vis = "pub")]
impl ThoriumMCP {
    /// List the pipelines available in a specific group.
    ///
    /// Returns projected summaries rather than full pipeline objects so that
    /// listing many pipelines does not bloat the agent's context.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "list_pipelines",
        description = "List the analysis pipelines available in a Thorium group. Returns a summary of each pipeline including its name, description, number of stages, and SLA. Use get_pipeline for full details about a specific pipeline."
    )]
    #[instrument(name = "ThoriumMCP::list_pipelines", skip(self, parts), err(Debug))]
    pub async fn list_pipelines(
        &self,
        Parameters(params): Parameters<ListPipelines>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // determine the limit
        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        // list pipelines in this group with full details
        let mut cursor = thorium.pipelines.list(&params.group).details().limit(limit);
        // fetch the data
        cursor.next().await?;
        // project each pipeline down to the fields an agent needs
        let summaries: Vec<PipelineSummary> = cursor
            .details
            .iter()
            .map(|pipeline| PipelineSummary {
                name: pipeline.name.clone(),
                group: pipeline.group.clone(),
                description: pipeline.description.clone(),
                stage_count: pipeline.order.len(),
                sla: pipeline.sla,
                trigger_count: pipeline.triggers.len(),
            })
            .collect();
        // check if there are more pipelines beyond this page
        let has_more = !cursor.exhausted;
        // serialize our projected pipelines
        let response = json!({
            "pipelines": &summaries,
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

    /// Get detailed information about a specific pipeline.
    ///
    /// Returns the full stage ordering so the agent can understand which
    /// images run in which order and plan reaction arguments accordingly.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_pipeline",
        description = "Get detailed information about a specific Thorium pipeline, including its stage ordering and images. Use this to understand the pipeline structure before creating a reaction."
    )]
    #[instrument(name = "ThoriumMCP::get_pipeline", skip(self, parts), err(Debug))]
    pub async fn get_pipeline(
        &self,
        Parameters(GetPipeline { group, pipeline }): Parameters<GetPipeline>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // get this pipeline
        let pipe = thorium.pipelines.get(&group, &pipeline).await?;
        // count the total number of images across all stages
        let image_count: usize = pipe.order.iter().map(|stage| stage.len()).sum();
        // project the pipeline to the fields an agent needs
        let detail = PipelineDetail {
            name: pipe.name,
            group: pipe.group,
            description: pipe.description,
            order: pipe.order,
            sla: pipe.sla,
            image_count,
        };
        // serialize our projected pipeline
        let serialized = serde_json::to_value(&detail).unwrap();
        // build our result
        let result = CallToolResult {
            content: vec![Content::json(&detail)?],
            structured_content: Some(serialized),
            is_error: Some(false),
            meta: None,
        };
        Ok(result)
    }
}
