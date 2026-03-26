//! Reaction management tools for the Thorium MCP server.
//!
//! Reactions are execution instances of pipelines — they are the primary
//! way agents trigger analysis and monitor progress. These tools enable
//! agents to:
//!
//! 1. [`create_reaction`](ThoriumMCP::create_reaction) — trigger a pipeline
//!    on one or more samples
//! 2. [`get_reaction`](ThoriumMCP::get_reaction) — poll reaction status
//! 3. [`list_reactions`](ThoriumMCP::list_reactions) — discover existing
//!    reactions for a pipeline
//! 4. [`get_reaction_logs`](ThoriumMCP::get_reaction_logs) — fetch execution
//!    logs for failure diagnosis

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use tracing::instrument;
use uuid::Uuid;

use crate::models::{ReactionListParams, ReactionRequest, ReactionStatus, StageLogs};

use super::ThoriumMCP;
use super::files::validate_sha256;

/// Default maximum number of reactions to return from a list operation.
const DEFAULT_LIST_LIMIT: u64 = 50;

/// Hard ceiling on reactions to prevent excessive responses.
const MAX_LIST_LIMIT: u64 = 500;

/// Default maximum number of log lines to return.
const DEFAULT_LOG_LIMIT: usize = 200;

/// Hard ceiling on log lines to prevent context bloat.
const MAX_LOG_LIMIT: usize = 1000;

/// The params needed to create a new reaction.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateReaction {
    /// The name of the group to create the reaction in.
    pub group: String,
    /// The name of the pipeline to run. Use list_pipelines to discover
    /// available pipelines.
    pub pipeline: String,
    /// The SHA256 hashes of the samples (files) to analyze.
    pub samples: Vec<String>,
    /// Optional tags to label this reaction for later discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional SLA in seconds. If not set, the pipeline's default SLA
    /// is used.
    #[serde(default)]
    pub sla: Option<u64>,
}

/// The params needed to get details about a reaction.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetReaction {
    /// The name of the group this reaction belongs to.
    pub group: String,
    /// The UUID of the reaction to get details for.
    pub reaction_id: Uuid,
}

/// The params needed to list reactions for a pipeline.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListReactions {
    /// The name of the group to list reactions from.
    pub group: String,
    /// The name of the pipeline to list reactions for.
    pub pipeline: String,
    /// Optional status filter. Valid values: "Created", "Started",
    /// "Completed", "Failed". If not set, all reactions are returned.
    #[serde(default)]
    pub status: Option<String>,
    /// Maximum number of reactions to return (default: 50).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Parse a status string into a [`ReactionStatus`].
fn parse_reaction_status(status: &str) -> Result<ReactionStatus, ErrorData> {
    match status {
        "Created" => Ok(ReactionStatus::Created),
        "Started" => Ok(ReactionStatus::Started),
        "Completed" => Ok(ReactionStatus::Completed),
        "Failed" => Ok(ReactionStatus::Failed),
        _ => Err(ErrorData {
            code: rmcp::model::ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid status '{}'. Valid values: Created, Started, Completed, Failed",
                status
            )
            .into(),
            data: None,
        }),
    }
}

/// The params needed to get logs for a reaction stage.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetReactionLogs {
    /// The name of the group this reaction belongs to.
    pub group: String,
    /// The UUID of the reaction to get logs for.
    pub reaction_id: Uuid,
    /// The name of the pipeline stage to get logs for. Use get_pipeline
    /// to see the stage names (image names in the pipeline order).
    pub stage: String,
    /// Maximum number of log lines to return (default: 200, max: 1000).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// A projected view of a reaction suitable for agent consumption.
///
/// Includes status and progress information but omits internal details
/// like job IDs, ephemeral files, and parent reaction linkage.
#[derive(Debug, Serialize, Deserialize)]
struct ReactionSummary {
    /// The UUID of this reaction.
    id: String,
    /// The group this reaction belongs to.
    group: String,
    /// The pipeline this reaction is executing.
    pipeline: String,
    /// The current status: Created, Started, Completed, or Failed.
    status: ReactionStatus,
    /// The index of the currently executing stage (0-based).
    current_stage: u64,
    /// The number of jobs completed in the current stage.
    current_stage_progress: u64,
    /// The total number of jobs in the current stage.
    current_stage_length: u64,
    /// The total number of jobs across all stages.
    jobs_count: usize,
    /// The sample SHA256 hashes being analyzed.
    samples: Vec<String>,
    /// The tags applied to this reaction.
    tags: Vec<String>,
    /// The user who created this reaction.
    creator: String,
    /// When the SLA expires (ISO 8601).
    sla: String,
}

/// Project a Reaction into a compact summary for agent consumption.
fn project_reaction(reaction: &crate::models::Reaction) -> ReactionSummary {
    ReactionSummary {
        id: reaction.id.to_string(),
        group: reaction.group.clone(),
        pipeline: reaction.pipeline.clone(),
        status: reaction.status.clone(),
        current_stage: reaction.current_stage,
        current_stage_progress: reaction.current_stage_progress,
        current_stage_length: reaction.current_stage_length,
        jobs_count: reaction.jobs.len(),
        samples: reaction.samples.clone(),
        tags: reaction.tags.clone(),
        creator: reaction.creator.clone(),
        sla: reaction.sla.to_rfc3339(),
    }
}

#[tool_router(router = reactions_router, vis = "pub")]
impl ThoriumMCP {
    /// Create a new reaction to run a pipeline on samples.
    ///
    /// A reaction is an execution instance of a pipeline. Use
    /// `list_pipelines` and `get_pipeline` to discover available pipelines
    /// and their structure before creating a reaction. Poll the reaction's
    /// status with `get_reaction` to monitor progress.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "create_reaction",
        description = "Create a new reaction to run an analysis pipeline on one or more samples. Use list_pipelines and get_pipeline first to understand pipeline structure. Returns a reaction_id that can be polled with get_reaction to monitor progress."
    )]
    #[instrument(name = "ThoriumMCP::create_reaction", skip(self, parts), err(Debug))]
    pub async fn create_reaction(
        &self,
        Parameters(CreateReaction {
            group,
            pipeline,
            samples,
            tags,
            sla,
        }): Parameters<CreateReaction>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // validate that at least one sample is provided
        if samples.is_empty() {
            return Err(ErrorData {
                code: rmcp::model::ErrorCode::INVALID_PARAMS,
                message: "At least one sample SHA256 must be provided".into(),
                data: None,
            });
        }
        // validate sha256 format for all samples
        for sample in &samples {
            validate_sha256(sample)?;
        }
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // build the reaction request
        let mut request = ReactionRequest::new(&group, &pipeline);
        request.samples = samples;
        request.tags = tags;
        request.sla = sla;
        // create the reaction
        let creation = thorium.reactions.create(&request).await?;
        // build the response, including group so the agent can call get_reaction
        let response = json!({
            "reaction_id": creation.id.to_string(),
            "group": group,
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

    /// Get the current status and progress of a reaction.
    ///
    /// Returns a projected summary with status, stage progress, sample
    /// list, and SLA. This is the primary polling target for monitoring
    /// reaction execution.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_reaction",
        description = "Get the current status and progress of a reaction. Returns status (Created, Started, Completed, Failed), stage progress, group, and sample list. Poll every 5-10 seconds after create_reaction until status is Completed or Failed. When completed, use get_sample_results on the samples. When failed, use get_pipeline to identify the failed stage from current_stage, then get_reaction_logs."
    )]
    #[instrument(name = "ThoriumMCP::get_reaction", skip(self, parts), err(Debug))]
    pub async fn get_reaction(
        &self,
        Parameters(GetReaction {
            group,
            reaction_id,
        }): Parameters<GetReaction>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // get the reaction
        let reaction = thorium.reactions.get(&group, reaction_id).await?;
        // project the reaction to agent-relevant fields
        let summary = project_reaction(&reaction);
        // serialize our projected reaction
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

    /// List reactions for a pipeline, optionally filtered by status.
    ///
    /// Returns projected summaries of reactions. Use the status filter to
    /// find failed reactions for diagnosis or completed reactions for result
    /// exploration.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "list_reactions",
        description = "List reactions for a pipeline in a group. Optionally filter by status (Created, Started, Completed, Failed). Returns projected summaries with status and progress. Use get_reaction for full details on a specific reaction."
    )]
    #[instrument(name = "ThoriumMCP::list_reactions", skip(self, parts), err(Debug))]
    pub async fn list_reactions(
        &self,
        Parameters(ListReactions {
            group,
            pipeline,
            status,
            limit,
        }): Parameters<ListReactions>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // determine the limit, capped at the hard ceiling and at least 1
        let limit = limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .min(MAX_LIST_LIMIT)
            .max(1);
        // parse the optional status filter
        let parsed_status = status
            .as_deref()
            .map(parse_reaction_status)
            .transpose()?;
        // build the cursor based on whether a status filter is set
        let mut cursor = match &parsed_status {
            Some(status) => thorium
                .reactions
                .list_status(&group, &pipeline, status)
                .details()
                .limit(limit),
            None => thorium
                .reactions
                .list(&group, &pipeline)
                .details()
                .limit(limit),
        };
        // fetch the data
        cursor.next().await?;
        // check if there are more reactions beyond this page
        let has_more = !cursor.exhausted;
        // project each reaction to agent-relevant fields
        let summaries: Vec<ReactionSummary> = cursor
            .details
            .iter()
            .map(project_reaction)
            .collect();
        // build the response
        let response = json!({
            "reactions": &summaries,
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

    /// Get execution logs for a specific stage in a reaction.
    ///
    /// Use this to diagnose failures. When a reaction has status "Failed",
    /// identify which stage failed by checking `get_reaction` progress,
    /// then fetch that stage's logs. Use `get_pipeline` to see the stage
    /// names (image names in the pipeline order).
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_reaction_logs",
        description = "Get execution logs for a specific stage in a reaction. Use get_pipeline to find stage names (image names). Use this to diagnose failures: check get_reaction for which stage failed, then fetch that stage's logs."
    )]
    #[instrument(name = "ThoriumMCP::get_reaction_logs", skip(self, parts), err(Debug))]
    pub async fn get_reaction_logs(
        &self,
        Parameters(GetReactionLogs {
            group,
            reaction_id,
            stage,
            limit,
        }): Parameters<GetReactionLogs>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // determine the limit, capped at the hard ceiling and at least 1
        let limit = limit
            .unwrap_or(DEFAULT_LOG_LIMIT)
            .min(MAX_LOG_LIMIT)
            .max(1);
        // build the list params for the logs request
        let params = ReactionListParams {
            cursor: 0,
            limit,
        };
        // fetch the logs
        let logs: StageLogs = thorium.reactions.logs(&group, &reaction_id, &stage, &params).await?;
        // heuristic: if we received exactly as many lines as we requested,
        // there are likely more. The logs API does not return an exhaustion
        // flag, so this is the best available signal.
        let truncated = logs.logs.len() >= limit;
        // build the response
        let response = json!({
            "reaction_id": reaction_id.to_string(),
            "stage": stage,
            "log_count": logs.logs.len(),
            "truncated": truncated,
            "logs": logs.logs,
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
