# Thorium MCP Layer — Architecture Design Document

**Status:** Implemented — Sprints 1-3 complete (2026-03-26)
**Purpose:** Architecture spec for Thorium's MCP agent-optimized tool layer.
Cross-referenced against the existing Rust codebase. Sprints 1-3 implemented and
verified end-to-end across six agent workflows.

---

## 1. Problem Statement

Thorium is a file analysis and data generation platform that orchestrates heterogeneous
Docker images (called **Images**) to perform binary analysis, forensic tooling, and
security research tasks at scale. Its existing REST API and web UI are designed for human
operators and dashboards: responses return full payloads, formats are varied (JSON, String,
Table, HTML, Markdown, XML, Disassembly, and more), and the data surface is optimized for
browsing rather than decision-making.

Agent workflows invert this model. Where humans browse then act, agents plan then fetch.
Every API call that returns a full payload is optimized for the wrong client.

Thorium already has a foundation-level MCP server (`api/src/routes/mcp/`) built on the
`rmcp` crate. It currently exposes 8 tools for documentation browsing, sample lookup,
image/pipeline listing, and tree traversal. However, these tools return unfiltered,
unprojected data and lack the agent-optimized response shaping needed for effective
autonomous operation.

The goal of this layer is to extend the existing MCP server with agent-optimized tools
that enable discovery, scoped content retrieval, and reaction management — without
modifying the Thorium backend.

---

## 2. Design Principles

1. **Content is always opt-in and always scoped.** No tool call returns full result
   content by default. Agents receive summaries first, then explicitly request specific
   tool results with optional truncation.

2. **Lists never include content.** List operations return projected field summaries.
   Content retrieval is a separate, explicit step.

3. **Errors and status are always cheap.** Reaction status and progress are returned as
   compact signal objects — the primary data agents act on.

4. **The transformation layer is purely additive.** The Thorium backend is not modified.
   All agent-optimized behavior lives in the MCP server and can evolve independently.
   Tools use the existing Rust client library (`crate::client::Thorium`) to access data.

5. **Tools are designed around agent decisions, not backend resources.** Tools map to
   what an agent needs to decide next, not to raw API responses.

6. **Tools use Thorium's native terminology.** Samples, Reactions, Pipelines, Images,
   Outputs — not generic terms like "jobs" or "artifacts."

---

## 3. Thorium Concept Map

Understanding Thorium's domain model is essential for correct tool design.

### 3.1 Core Entities

| Concept | Description | Access Pattern |
|---|---|---|
| **Sample** | A file uploaded for analysis, identified by SHA256 hash | `GET /samples/{sha256}` |
| **Image** | A Docker image that performs analysis (e.g., YARA scanner, strings extractor) | Scoped to a Group |
| **Pipeline** | A reusable blueprint defining ordered stages of Images to execute | Scoped to a Group |
| **Reaction** | A single execution instance of a Pipeline on specific Samples | The user-facing orchestration unit |
| **Job** | A single Image execution within a Reaction (internal, not user-created) | Managed by Thorium's scheduler |
| **Output** | A tool result from analyzing a Sample, stored as `serde_json::Value` | Accessed by SHA256 + tool name |
| **Group** | Multi-tenancy boundary — all resources are scoped to one or more Groups | Required for most API calls |

### 3.2 Data Flow

```
User/Agent creates a Reaction
  -> specifies: Group, Pipeline, Samples (SHA256s), optional Args
  -> Thorium creates Jobs for each Pipeline stage
  -> Scaler assigns Jobs to workers (K8s pods, bare metal, etc.)
  -> Agent (worker) executes Image container with Sample data
  -> Agent uploads results as Outputs (indexed by SHA256 + tool name)
  -> Reaction progresses through stages until Completed or Failed
```

### 3.3 Key Differences from Pre-Review Design

| Original Assumption | Actual Thorium Model |
|---|---|
| "Jobs" are the user-facing orchestration unit | **Reactions** are user-facing; Jobs are internal |
| "Artifacts" with stable artifact_id | **Outputs** accessed by Sample SHA256 + tool name |
| Artifact formats: CSV, JSON, TXT, binary | Output `display_type`: Json, String, Table, Image, Custom, Disassembly, Html, Markdown, Hidden, Xml |
| `create_job(tool, params)` | `create_reaction(group, pipeline, samples, args)` |
| `chain_jobs(steps)` | Pipelines already define ordered stages |
| Results accessed by job_id | Results accessed by SHA256 + tool name filter |
| ArtifactMeta with preview/schema | Output with `result: serde_json::Value` + `display_type` |

### 3.4 State Machines

**Reaction status:** `Created -> Started -> Completed | Failed`

**Job status (internal):** `Created -> Running -> Completed | Failed | Sleeping`
(Sleeping is for generator jobs that return to be respawned.)

**Result storage:**
- Small results (< 1 MiB): stored inline in ScyllaDB as `serde_json::Value`
- Large results: stored as files in S3, referenced by path
- Children: extracted/unpacked files uploaded as new Samples

---

## 4. Architecture Overview

```
[Thorium Backend (REST API)]
      |
      |  Rust client library (crate::client::Thorium)
      v
[MCP Server (api/src/routes/mcp/)]
  - Auth:  Token extracted from Authorization header per request
  - Transport:  StreamableHttp with LocalSessionManager
  - Framework:  rmcp crate with #[tool_router] macros
      |
      |  Response projection (field filtering per resource type)
      |  Content truncation (optional max_chars on result content)
      |  Summary generation (output maps -> tool summary maps)
      v
[MCP Tools — 3 Tiers]
  - Discovery tier     (groups, pipelines, images, documentation)
  - Analyst tier       (samples, results, search)
  - Operator tier      (reactions: create, monitor, diagnose)
  - Synthesis tier     (deferred — inner LLM calls for cross-result correlation)
```

### 4.1 Existing Implementation

The MCP server already exists at `api/src/routes/mcp/` with this architecture:

```rust
pub struct ThoriumMCP {
    conf: McpConfig,
    tool_router: ToolRouter<Self>,
}

// Each tool module uses #[tool_router] macro to define tools
// Auth: token from Authorization header -> Thorium client per request
// All tools follow: extract params -> build client -> call API -> project response
```

**Currently implemented tools (8):**

| Tool | Module | Status |
|---|---|---|
| `get_docs_toc` | `mcp/docs.rs` | Well implemented, tested |
| `get_doc_page` | `mcp/docs.rs` | Well implemented, path traversal protection |
| `search_docs` | `mcp/docs.rs` | Well implemented, snippet extraction |
| `get_sample` | `mcp/files.rs` | Returns full Sample — needs field projection |
| `get_sample_results` | `mcp/files.rs` | Returns ALL results unfiltered — needs overhaul |
| `list_images` | `mcp/images.rs` | Returns full Image structs (very large) — needs projection |
| `list_pipelines` | `mcp/pipelines.rs` | Returns full structs, has wrong docstring |
| `start_tree` | `mcp/trees.rs` | Returns full tree — can be very large |

---

## 5. Tool Surface

Tools are organized into four tiers. Each tier has a distinct context budget contract.

### 5.1 Discovery Tier

Discovery tools help the agent understand what is available in Thorium before taking
action. They return projected metadata, never content.

```
list_groups()
    -> [{
        name: str,
        description: str | null
    }]
    # Agent must discover groups first — group is required for most operations.

list_pipelines(group: str, limit?: u64)
    -> [{
        name: str,
        group: str,
        description: str | null,
        stage_count: usize,          # number of stages in the pipeline
        sla: u64,                    # default SLA in seconds
        trigger_count: usize         # number of auto-triggers configured
    }]
    # Projected fields only. No full Pipeline structs.

get_pipeline(group: str, pipeline: str)
    -> {
        name: str,
        group: str,
        description: str | null,
        order: [[str]],              # stages and their images
        sla: u64,
        image_count: usize           # total images across all stages
    }
    # Detail view for understanding pipeline structure before creating reactions.

list_images(group: str, limit?: u64)
    -> [{
        name: str,
        group: str,
        description: str | null,
        timeout: u64 | null,         # max execution time in seconds
        generator: bool              # whether this is a generator image
    }]
    # Projected fields only. No resource limits, volumes, security context, etc.

get_image(group: str, image: str)
    -> {
        name: str,
        group: str,
        description: str | null,
        timeout: u64 | null,
        generator: bool,
        dependencies: {              # what this image needs as input
            needs_samples: bool,
            needs_repos: bool,
            needs_prior_results: bool,
            result_images: [str]     # which images' results it depends on
        },
        display_type: str            # how results are rendered (Json, String, etc.)
    }
    # Detail view for understanding what an image does and what it needs.

get_docs_toc()                       # EXISTING — no changes needed
get_doc_page(path: str)              # EXISTING — no changes needed
search_docs(query: str, ...)         # EXISTING — no changes needed
```

### 5.2 Analyst Tier (Result Exploration)

Analyst tools provide structured access to analysis results. Content retrieval follows
a two-step pattern: summary first, then scoped content fetch.

```
get_sample(sha256: str)
    -> {
        sha256: str,
        groups: [str],
        tags: [str],
        size: u64,
        uploaded: str,               # ISO timestamp
        description: str | null
    }
    # EXISTING — improved with field projection.

get_sample_results(sha256: str, tools?: [str], groups?: [str], include_hidden?: bool)
    -> {
        sha256: str,
        tool_count: usize,
        tools: {
            "<tool_name>": {
                result_count: usize,
                display_type: str,
                latest_uploaded: str,    # ISO timestamp
                has_files: bool
            }
        }
    }
    # MODIFIED — returns summary map instead of full content.
    # Agent uses this to discover WHAT tools have run, then decides which to fetch.

get_sample_result(sha256: str, tool: str, group?: str, max_chars?: usize)
    -> {
        sha256: str,
        tool: str,
        result_count: usize,
        results: [{
            id: str,
            cmd: str | null,
            uploaded: str,
            display_type: str,
            result: Value,               # the actual result content
            files: [str],
            truncated: bool              # true if max_chars caused truncation
        }]
    }
    # NEW — scoped content fetch for a single tool's results.
    # max_chars truncates the JSON serialization of each result value.

search_results(query: str, groups: [str], max_results?: usize)
    -> {
        query: str,
        hit_count: usize,
        hits: [{
            sha256: str | null,          # populated for sample results
            url: str | null,             # populated for repo results
            group: str | null,
            index: str,                  # Elasticsearch index name
            score: f64 | null,
            excerpt: str                 # bounded to 500 chars
        }]
    }
    # Elasticsearch search with Lucene syntax. Query length capped at 1000
    # chars. Results capped at 100. Note: tool name is not available from
    # the ES document — use get_sample_results to discover which tools
    # produced results for a matched sample.

start_tree(
    samples?: [str],                     # at least one input required
    repos?: [str],
    entities?: [uuid],
    tags?: [{ key: [values] }],
    groups?: [str],                      # scope traversal to groups
    limit?: usize                        # max nodes to return (default 25, max 100)
)
    -> {
        tree_id: str,
        node_count: usize,
        total_nodes_in_tree: usize,
        truncated: bool,
        edge_count: usize,
        growable_count: usize,
        nodes: [{
            node_id: u64,
            node_type: "sample" | "repo" | "entity" | "tag",
            identifier: str,             # SHA256, URL, UUID, or tag string
            label: str                   # filename, org/name, "Name (Kind)", tag desc
        }],
        edges: [{
            from_node: u64,
            to_node: u64,
            relationship: str            # e.g. "Origin: Unpacked", "Association: FileFor"
        }]
    }
    # OVERHAULED — Projects full TreeNode objects to lightweight summaries.
    # Deterministic: initial nodes sorted first, then by node_id.
    # Traversal depth fixed at 50 (backend default); limit only caps output.
    # SHA256 validation on sample inputs. Input validation requires at least
    # one starting point.
```

### 5.3 Operator Tier (Reaction Management)

Operator tools enable agents to trigger analysis pipelines and monitor their progress.
They return compact status signals, never raw content.

```
create_reaction(
    group: str,
    pipeline: str,
    samples: [str],                  # SHA256 hashes of files to analyze
    tags?: [str],
    sla?: u64                        # deadline in seconds
)
    -> { reaction_id: str, group: str }
    # Creates a new Reaction (pipeline execution instance).
    # The Pipeline defines which Images run in which order.
    # Returns group alongside reaction_id for immediate polling.
    # SHA256 validation on all sample inputs.
    # Note: per-image args are deferred — GenericJobArgs does not derive
    # JsonSchema. Reactions use pipeline default arguments.

get_reaction(group: str, reaction_id: uuid)
    -> {
        id: str,
        group: str,                      # included for parameter threading
        pipeline: str,
        status: "Created" | "Started" | "Completed" | "Failed",
        current_stage: u64,
        current_stage_progress: u64,     # jobs completed in current stage
        current_stage_length: u64,       # total jobs in current stage
        jobs_count: usize,
        samples: [str],
        tags: [str],
        creator: str,
        sla: str                         # ISO timestamp
    }
    # Projected reaction status — the primary polling target for agents.
    # Poll every 5-10 seconds. When completed, use get_sample_results on
    # the samples. When failed, map current_stage to get_pipeline order
    # and use get_reaction_logs for the failed stage's image name.

list_reactions(
    group: str,
    pipeline: str,
    status?: str,                    # "Created", "Started", "Completed", "Failed"
    limit?: u64                      # default 50, max 500
)
    -> {
        reactions: [{
            id: str,
            group: str,
            pipeline: str,
            status: str,
            current_stage: u64,
            current_stage_progress: u64,
            current_stage_length: u64,
            jobs_count: usize,
            samples: [str],
            tags: [str],
            creator: str,
            sla: str
        }],
        has_more: bool
    }
    # List reactions for a pipeline, optionally filtered by status.
    # Uses same ReactionSummary projection as get_reaction.

get_reaction_logs(
    group: str,
    reaction_id: uuid,
    stage: str,                      # image name from get_pipeline order
    limit?: usize                    # default 200, max 1000
)
    -> {
        reaction_id: str,
        stage: str,
        log_count: usize,
        logs: [str],                 # raw log lines from the stage
        truncated: bool              # heuristic: true if log_count >= limit
    }
    # Fetch execution logs for a specific pipeline stage.
    # Critical for diagnosing failures. Stage name is the image name
    # from get_pipeline's order array at the current_stage index.
```

### 5.4 Synthesis Tier (Deferred)

Synthesis tools perform internal LLM calls inside the MCP server, returning conclusions
rather than raw data. These are high-value for forensics but require significant new
infrastructure and are deferred until the base tiers prove their value.

**Deferred tools:**
- `correlate_results(sha256, tools, question)` — Cross-tool correlation with cited evidence
- `diagnose_reaction(group, reaction_id)` — Failure root-cause analysis from logs and status
- `summarize_results(sha256, tools)` — Key findings summary across tool outputs

These will use a smaller/faster model than the outer agent where possible.

---

## 6. Workflow Examples

### 6.1 Automated Binary Analysis Pipeline (Operator Mode)

```
1. list_groups()
   -> { groups: [{ name: "malware-lab" }, { name: "static-analysis" }], has_more: false }

2. list_pipelines(group="static-analysis")
   -> { pipelines: [{ name: "default-static", stage_count: 5, sla: 3600, ... }], has_more: false }

3. get_pipeline(group="static-analysis", pipeline="default-static")
   -> { order: [["strings"], ["trid"], ["capa", "yara-scanner"], ...], image_count: 5 }

4. create_reaction(group="static-analysis", pipeline="default-static", samples=["abc123..."])
   -> { reaction_id: "550e8400-...", group: "static-analysis" }

5. get_reaction(group="static-analysis", reaction_id="550e8400...")  [poll every 5-10s]
   -> { id: "550e8400-...", group: "static-analysis", status: "Completed",
        current_stage: 4, samples: ["abc123..."], ... }

6. get_sample_results(sha256="abc123...")
   -> { sha256: "abc123...", tool_count: 5,
        tools: { "strings": { result_count: 1, display_type: "String", ... }, ... } }

7. get_sample_result(sha256="abc123...", tool="capa", max_chars=4000)
   -> { sha256: "abc123...", tool: "capa", result_count: 1,
        results: [{ result: {...}, truncated: false }] }
```

The agent made 7 tool calls. It discovered the environment, triggered analysis,
monitored completion, and fetched only the specific results it needed.

### 6.2 Investigating Existing Results (Analyst Mode)

```
1. search_results(query="mimikatz OR credential.dump", groups=["incident-response"])
   -> { query: "...", hit_count: 3,
        hits: [{ sha256: "abc...", group: "incident-response", score: 12.5,
                 excerpt: "...mimikatz credential dump..." }] }

2. get_sample(sha256="abc...")
   -> { sha256: "abc...", groups: ["incident-response"],
        tags: { "malware_family": ["mimikatz"] }, name: "suspicious.exe", ... }

3. get_sample_results(sha256="abc...")
   -> { tool_count: 4, tools: { "yara-scanner": { result_count: 1, ... },
        "strings": { result_count: 1, ... }, "capa": { result_count: 1, ... } } }

4. get_sample_result(sha256="abc...", tool="yara-scanner")
   -> { results: [{ result: { matched_rules: [...] }, truncated: false }] }

5. get_sample_result(sha256="abc...", tool="capa", max_chars=3000)
   -> { results: [{ result: { capabilities: [...] }, truncated: true }] }
```

The agent searched across the corpus, found a suspicious sample, discovered what tools
had run on it, and selectively fetched the results it cared about.

### 6.3 Diagnosing a Failed Reaction (Operator Mode)

```
1. list_reactions(group="static-analysis", pipeline="default-static", status="Failed", limit=5)
   -> { reactions: [{ id: "fail-001", group: "static-analysis",
        status: "Failed", current_stage: 2, ... }], has_more: false }

2. get_reaction(group="static-analysis", reaction_id="fail-001")
   -> { id: "fail-001", group: "static-analysis", pipeline: "default-static",
        status: "Failed", current_stage: 2, current_stage_progress: 0,
        current_stage_length: 1 }

3. get_pipeline(group="static-analysis", pipeline="default-static")
   -> { order: [["strings"], ["capa"], ["yara-scanner"]] }
   # Failed at stage 2 = "capa" (0-indexed)

4. get_reaction_logs(group="static-analysis", reaction_id="fail-001", stage="capa", limit=50)
   -> { reaction_id: "fail-001", stage: "capa", log_count: 12,
        truncated: false, logs: ["ERROR: insufficient memory..."] }
```

The agent diagnosed a failure in 4 tool calls without examining any result content.

---

## 7. Response Token Budget Guidelines

These are targets, not hard limits. The MCP server should enforce ceiling limits on all
responses to prevent accidental context bloat.

| Tool Category             | Target Token Budget | Hard Ceiling |
|---------------------------|--------------------:|-------------:|
| Status/signal tools       |           ~100 tok  |      500 tok |
| List operations           |       ~50 tok/item  |     2000 tok |
| Result summary            |           ~200 tok  |     1000 tok |
| Scoped result content     |         ~1000 tok   |     4000 tok |
| Search results            |       ~100 tok/hit  |     2000 tok |
| Reaction logs             |           ~500 tok  |     2000 tok |
| Synthesis output          |           ~500 tok  |     1500 tok |

Tools that would exceed their hard ceiling should truncate with a `truncated: true` flag
and guidance on how to scope the request further (e.g., use `max_chars`, filter by tool).

---

## 8. Implementation Priorities

### Sprint 1 — Discovery + Result Exploration (Complete)

- [x] `list_groups` — new file `mcp/groups.rs`
- [x] Fix `list_pipelines` — field projection, fix docstring, add limit param
- [x] Fix `list_images` — field projection, add limit param
- [x] `get_pipeline` — add to `mcp/pipelines.rs`
- [x] `get_image` — add to `mcp/images.rs` (includes `used_by` and dependency summary)
- [x] Modify `get_sample_results` — add tool/group filtering, return summary map
- [x] `get_sample_result` — new tool in `mcp/files.rs` with `max_chars` truncation
- [x] `search_results` — new file `mcp/search.rs` using Elasticsearch
- [x] Improve `get_sample` — SampleSummary projection with flattened tags

**Verified:** End-to-end discovery and analyst workflows confirmed.

### Sprint 2 — Reaction Management (Complete)

- [x] `create_reaction` — new file `mcp/reactions.rs` (args deferred)
- [x] `get_reaction` — projected status with polling guidance in description
- [x] `list_reactions` — with status filtering and `has_more` signal
- [x] `get_reaction_logs` — with truncation heuristic and limit cap

**Verified:** End-to-end operator workflows confirmed including failure diagnosis.

### Sprint 3 — Polish + Hardening (Complete)

- [x] `truncate_json_value` utility for `max_chars` with unit tests
- [x] `get_sample` SampleSummary projection (moved from Sprint 1 scope)
- [x] Overhaul `start_tree` — node projection, limit, groups, deterministic selection,
      `describe_relationship()`, SHA256 validation, input validation
- [x] MAX_LIST_LIMIT (500) ceiling on all list tools (pipelines, images, reactions)
- [x] `.max(1)` floor clamping on all list and search tools
- [x] `MAX_SEARCH_RESULTS` (100) and `MAX_QUERY_LENGTH` (1000) on search
- [x] SHA256 validation on all tools that accept sample hashes
- [x] `has_more` pagination signal on all list tools
- [x] `to_str().unwrap()` panic fix in auth header extraction
- [x] `content`/`structured_content` shape alignment across all tools
- [x] Tool description polish — next-step guidance, entry point clarity
- [x] End-to-end workflow verification across 6 workflows, 17 tools

**Verified:** All design principles upheld. No data flow breaks.

### Sprint 4 — Synthesis (Deferred)

- [ ] `correlate_results` (inner LLM call)
- [ ] `diagnose_reaction` (inner LLM on error state + logs)
- [ ] `summarize_results`
- [ ] Semantic search option for `search_results`

**Validation:** Agent can answer "what happened in this incident" from results alone,
with cited evidence.

---

## 9. Codebase Review Findings

These questions from the original design document have been answered by inspecting the
Thorium codebase.

**Result access:**
- Tool outputs are stored as `Output` structs with `result: serde_json::Value` in
  ScyllaDB (< 1 MiB) or as files in S3 (> 1 MiB). Accessed via
  `GET /samples/{sha256}/results` with optional `?tools=X&groups=Y` filtering.
- Each Output has a UUID (`Output.id`), but the primary access pattern is by
  sample SHA256 + tool name. There is no single "artifact ID" lookup.
- Results are immutable once produced.
- Outputs are grouped in `OutputMap { results: HashMap<String, Vec<Output>> }` keyed
  by tool name.

**Reaction model:**
- State machine: `Created -> Started -> Completed | Failed`
- Individual jobs: `Created -> Running -> Completed | Failed | Sleeping`
- No webhook system. Event handler watches a Redis stream with 3-second trail.
  MCP tools should use polling via `get_reaction`.
- Errors surface as `Failed` status on the Reaction. Logs available per-stage via
  `get_reaction_logs`.

**Tool registry:**
- Full tool manifest exists at `tools/toolbox.json` (190 KB) with per-tool JSON
  definitions under `tools/images/`.
- Images declare output formats via `display_type` and `output_collection` config.
- Rich dependency declarations: samples, repos, ephemeral files, prior results.

**Auth and multi-tenancy:**
- Token-based authentication (Bearer token in Authorization header).
- Group-based multi-tenancy — all resources scoped to Groups.
- Role hierarchy: Admin > Analyst > Developer > User (system-level);
  Owner > Manager > User > Monitor (per-group).

**Scale:**
- Results typically < 1 MiB (inline JSON). Large results stored as S3 files.
- Pipelines typically 1-20 stages, each stage running 1-5 images in parallel.
- Common image timeouts: 60-300 seconds.

---

## 10. Design Decisions Deferred

| Approach | Rationale for Deferral |
|---|---|
| Synthesis tools (inner LLM calls) | High value but requires new infrastructure. Defer until base tools prove their value with real agent workflows. |
| RAG/semantic search | Strong for retrospective analysis. Deferred until text search via Elasticsearch proves insufficient. |
| Explicit state machine workflows | High value for SOPs. Revisit once 2-3 common analyst workflows are well-understood from agent usage. |
| Reaction caching/piping | `ReactionCache` system exists in Thorium but is complex. Defer until agent workflows demonstrate the need. |
| Code Mode (OpenAPI-driven) | Requires clean OpenAPI spec and hardened sandbox. High value long-term once spec exists. |
| Dynamic tool loading | Low complexity. Revisit if tool count grows large enough to impact context at session init. |

---

## 11. Implementation Reference

### Tool Pattern (Rust)

All MCP tools follow this established pattern in the codebase:

```rust
/// Parameter struct with JsonSchema for MCP input validation
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ToolParams {
    /// Description shown in MCP tool schema
    pub required_field: String,
    #[serde(default)]
    pub optional_field: Option<String>,
}

#[tool_router(router = module_router, vis = "pub")]
impl ThoriumMCP {
    #[tool(
        name = "tool_name",
        description = "Clear description for agents."
    )]
    #[instrument(name = "ThoriumMCP::tool_name", skip(self, parts), err(Debug))]
    pub async fn tool_name(
        &self,
        Parameters(params): Parameters<ToolParams>,
        RmcpExtension(parts): RmcpExtension<axum::http::request::Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let thorium = self.conf.client(&parts).await?;
        // ... call thorium client methods, project response fields ...
        let serialized = serde_json::to_value(&projected).unwrap();
        Ok(CallToolResult {
            content: vec![Content::json(&projected)?],
            structured_content: Some(serialized),
            is_error: Some(false),
            meta: None,
        })
    }
}
```

### Key Client Methods

| Method | Returns | Used By |
|---|---|---|
| `thorium.groups.list()` | `Cursor<Group>` | `list_groups` |
| `thorium.pipelines.list(&group)` | `Cursor<Pipeline>` | `list_pipelines` |
| `thorium.pipelines.get(&group, &name)` | `Pipeline` | `get_pipeline` |
| `thorium.images.list(&group)` | `Cursor<Image>` | `list_images` |
| `thorium.images.get(&group, &name)` | `Image` | `get_image` |
| `thorium.files.get(&sha256)` | `Sample` | `get_sample` |
| `thorium.files.get_results(&sha256, &params)` | `OutputMap` | `get_sample_results`, `get_sample_result` |
| `thorium.search.search(&opts)` | `Cursor<ElasticDoc>` | `search_results` |
| `thorium.reactions.create(&req)` | `ReactionCreation` | `create_reaction` |
| `thorium.reactions.get(&group, id)` | `Reaction` | `get_reaction` |
| `thorium.reactions.list(&group, &pipeline)` | `Cursor<Reaction>` | `list_reactions` |
| `thorium.reactions.logs_cursor(&group, &id, &stage)` | `LogsCursor` | `get_reaction_logs` |

### Router Registration

New tool modules are registered in `ThoriumMCP::new()`:

```rust
Self {
    conf: mcp_conf,
    tool_router: Self::sample_router()
        + Self::images_router()
        + Self::pipelines_router()
        + Self::tree_router()
        + Self::docs_router()
        + Self::groups_router()      // NEW
        + Self::search_router()      // NEW
        + Self::reactions_router(),  // NEW (Sprint 2)
}
```
