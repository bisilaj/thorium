## Summary

Extends Thorium's MCP server with 10 new agent-optimized tools and improves 5 existing tools, bringing the total to 18. This enables LLM agents to discover Thorium's capabilities, explore analysis results, trigger pipelines with custom arguments, monitor reactions, diagnose failures, cancel reactions, and traverse data relationships -- all through the MCP protocol.

The implementation follows six design principles: content is opt-in, lists never include content, errors are cheap, the layer is additive (no backend modifications), tools are designed around agent decisions, and all tools use Thorium's native terminology.

Tested locally on Minithor with 16 of 18 tools confirmed working against a live Thorium instance. End-to-end chain verified: search_results -> get_sample -> get_sample_results.

### New tools

- **`list_groups`** -- Discover accessible groups (entry point for all workflows)
- **`get_pipeline`** -- Pipeline structure with stage ordering and image list
- **`get_image`** -- Image capabilities, dependencies, output format, and usage
- **`get_sample_result`** -- Fetch specific tool's result content with optional `max_chars` truncation
- **`search_results`** -- Elasticsearch search with Lucene syntax, bounded excerpts, SHA256/group extraction from _source, highlight, and _id fields; Kibana tag stripping
- **`create_reaction`** -- Trigger a pipeline on samples with optional per-image argument overrides, returns reaction_id for polling
- **`get_reaction`** -- Poll reaction status with stage progress and next-step guidance
- **`list_reactions`** -- List reactions with status filtering (Created/Started/Completed/Failed)
- **`get_reaction_logs`** -- Fetch stage execution logs for failure diagnosis
- **`cancel_reaction`** -- Cancel (delete) a reaction that is no longer needed

### Improved existing tools

- **`list_pipelines`** -- Field projection (was returning full Pipeline structs), limit param, fixed copy-paste docstring
- **`list_images`** -- Field projection (was returning full Image structs), limit param
- **`get_sample`** -- Projected to SampleSummary (was returning full Sample with all submissions/comments)
- **`get_sample_results`** -- Returns tool summary map (counts, types, timestamps) instead of dumping all result content; adds tool/group/hidden filtering
- **`start_tree`** -- Complete overhaul: projects TreeNode to lightweight summaries, deterministic node selection, `describe_relationship()` for clean edge labels, groups/limit params, input validation, SHA256 validation
- **`get_docs_toc`** -- Fixed structuredContent to return object instead of bare array
- **`search_docs`** -- Fixed structuredContent to return object instead of bare array

### Cross-cutting improvements

- `has_more` pagination signal on all list tools
- `MAX_LIST_LIMIT` ceiling (500) and `.max(1)` floor on all list/search tools
- SHA256 format validation on all tools that accept sample hashes
- Search query length cap (1000 chars) and result ceiling (100 hits)
- Search field extraction fallback chain: _source -> highlight -> _id parsing
- Kibana highlight tag stripping from search excerpts
- `content` and `structured_content` fields aligned across all tools
- Fixed `to_str().unwrap()` panic on malformed auth headers in mcp.rs
- `truncate_json_value` utility with unit tests including multi-byte char handling

## Test plan

- [x] `cargo +nightly build -p thorium-api --lib` compiles cleanly (zero errors, zero warnings in MCP code)
- [x] `cargo +nightly test -p thorium-api --lib -- mcp` passes all 49 tests (31 new + 18 existing)
- [x] Multiple independent code reviews with fresh-context agents across all sprint changes
- [x] End-to-end data flow verification across 6 agent workflows
- [x] Live testing on Minithor (3 rounds): 16 of 18 tools confirmed working
- [x] End-to-end chain verified: search_results -> get_sample -> get_sample_results
- [ ] Testing with real pipelines/images on staging (create_reaction, get_reaction, get_reaction_logs, cancel_reaction)

## Known issues

- **`start_tree` deserialization bug** -- Pre-existing issue in the Thorium client library where `Trees::start()` fails to deserialize the response from `POST /api/trees/`. The REST endpoint returns valid JSON but the client's `serde` deserialization fails. See `mcp-design-docs/BUG_REPORT_START_TREE.md` for detailed reproduction steps and diagnosis guide. Not fixable in the MCP layer.

## Known deferred items

- **`grow_tree` tool** -- Blocked until `start_tree` deserialization is fixed
- **`get_repo` / `get_entity` tools** -- Tree discovers these nodes but no detail tools yet
- **Cross-pipeline `list_reactions`** -- Requires a pipeline parameter; cannot list all failed reactions across a group in one call
- **Synthesis tier** (Sprint 4) -- `correlate_results`, `diagnose_reaction`, `summarize_results` require inner LLM infrastructure
