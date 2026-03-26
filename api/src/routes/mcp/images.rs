//! Image discovery tools for the Thorium MCP server.
//!
//! Images are Docker containers that perform analysis tasks. An agent uses
//! these tools to understand what analysis capabilities are available and
//! what inputs each image expects.

use rmcp::ErrorData;
use rmcp::handler::server::tool::Extension as RmcpExtension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde_json::json;
use tracing::instrument;

use crate::models::DependencyPassStrategy;

use super::ThoriumMCP;

/// Default maximum number of images to return from a list operation.
const DEFAULT_LIST_LIMIT: u64 = 100;

/// Hard ceiling on images to prevent excessive responses.
const MAX_LIST_LIMIT: u64 = 500;

/// The params needed to list images in a group.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListImages {
    /// The name of the group to list images for.
    pub group: String,
    /// Maximum number of images to return (default: 100).
    #[serde(default)]
    pub limit: Option<u64>,
}

/// The params needed to get details about a specific image.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetImage {
    /// The name of the group this image belongs to.
    pub group: String,
    /// The name of the image to get details for.
    pub image: String,
}

/// A projected view of an image suitable for list results.
///
/// Only the fields an agent needs for discovery are included; resource
/// limits, volumes, security contexts, and other operational details are
/// omitted.
#[derive(Debug, Serialize, Deserialize)]
struct ImageSummary {
    /// The name of this image.
    name: String,
    /// The group this image belongs to.
    group: String,
    /// A human-readable description of the image, if one was set.
    description: Option<String>,
    /// The maximum execution time in seconds, if one was set.
    timeout: Option<u64>,
    /// Whether this is a generator image that can spawn sub-reactions.
    generator: bool,
}

/// A projected dependency summary for an image.
#[derive(Debug, Serialize, Deserialize)]
struct ImageDependencySummary {
    /// Whether this image requires samples (files) as input.
    needs_samples: bool,
    /// Whether this image requires git repositories as input.
    needs_repos: bool,
    /// Whether this image requires prior results from other images.
    needs_prior_results: bool,
    /// The names of images whose results this image depends on.
    result_images: Vec<String>,
}

/// A detailed view of an image for understanding its capabilities.
#[derive(Debug, Serialize, Deserialize)]
struct ImageDetail {
    /// The name of this image.
    name: String,
    /// The group this image belongs to.
    group: String,
    /// A human-readable description of the image, if one was set.
    description: Option<String>,
    /// The maximum execution time in seconds, if one was set.
    timeout: Option<u64>,
    /// Whether this is a generator image that can spawn sub-reactions.
    generator: bool,
    /// What this image needs as input.
    dependencies: ImageDependencySummary,
    /// How results from this image are rendered (e.g. "Json", "String", "Table").
    display_type: String,
    /// The pipelines that use this image.
    used_by: Vec<String>,
}

#[tool_router(router = images_router, vis = "pub")]
impl ThoriumMCP {
    /// List the images available in a specific group.
    ///
    /// Returns projected summaries rather than full image objects so that
    /// listing many images does not bloat the agent's context.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "list_images",
        description = "List the analysis images (tools) available in a Thorium group. Returns a summary of each image including its name, description, timeout, and whether it is a generator. Use get_image for full details about a specific image."
    )]
    #[instrument(name = "ThoriumMCP::list_images", skip(self, parts), err(Debug))]
    pub async fn list_images(
        &self,
        Parameters(params): Parameters<ListImages>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // determine the limit, capped at the hard ceiling and at least 1
        let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT).max(1);
        // list images in this group with full details
        let mut cursor = thorium.images.list(&params.group).details().limit(limit);
        // fetch the data
        cursor.next().await?;
        // project each image down to the fields an agent needs
        let summaries: Vec<ImageSummary> = cursor
            .details
            .iter()
            .map(|image| ImageSummary {
                name: image.name.clone(),
                group: image.group.clone(),
                description: image.description.clone(),
                timeout: image.timeout,
                generator: image.generator,
            })
            .collect();
        // check if there are more images beyond this page
        let has_more = !cursor.exhausted;
        // serialize our projected images
        let response = json!({
            "images": &summaries,
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

    /// Get detailed information about a specific image.
    ///
    /// Returns the image's dependencies, display type, and which pipelines
    /// use it. Helps the agent understand what an image does, what inputs
    /// it expects, and how its results are formatted.
    ///
    /// # Arguments
    ///
    /// * `parameters` - The parameters required for this tool
    /// * `parts` - The request parts required to get a token for this tool
    #[tool(
        name = "get_image",
        description = "Get detailed information about a specific Thorium image (analysis tool), including its dependencies, output format, and which pipelines use it. Use this to understand what an image needs as input before creating a reaction."
    )]
    #[instrument(name = "ThoriumMCP::get_image", skip(self, parts), err(Debug))]
    pub async fn get_image(
        &self,
        Parameters(GetImage { group, image }): Parameters<GetImage>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        // get a thorium client
        let thorium = self.conf.client(&parts).await?;
        // get this image
        let img = thorium.images.get(&group, &image).await?;
        // summarize the image's dependencies
        let deps = ImageDependencySummary {
            needs_samples: img.dependencies.samples.strategy != DependencyPassStrategy::Disabled,
            needs_repos: img.dependencies.repos.strategy != DependencyPassStrategy::Disabled,
            needs_prior_results: !img.dependencies.results.images.is_empty(),
            result_images: img.dependencies.results.images.clone(),
        };
        // project the image to the fields an agent needs
        let detail = ImageDetail {
            name: img.name,
            group: img.group,
            description: img.description,
            timeout: img.timeout,
            generator: img.generator,
            dependencies: deps,
            display_type: img.display_type.as_str().to_owned(),
            used_by: img.used_by,
        };
        // serialize our projected image
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
