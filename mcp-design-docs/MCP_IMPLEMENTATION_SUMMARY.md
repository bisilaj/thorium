# MCP Agent Tools — Implementation Summary

**Date:** 2026-04-01
**Branch:** `feat/mcp-agent-tools` (forked from `feat/mcp-docs-tools`)
**Fork:** `github.com/bisilaj/thorium`

---

## What Was Built

Extended Thorium's MCP server with 10 new agent-optimized tools, improved 5
existing tools, and hardened the entire tool surface. The MCP layer sits
between LLM agents (MCP clients such as OpenCode, Cursor, etc.) and the Thorium REST API,
transforming human-optimized responses into agent-optimized ones.

### Design Principles

1. **Content is opt-in** — no tool returns result content by default
2. **Lists never include content** — list tools return projected summaries
3. **Errors are cheap** — compact error responses with trace IDs
4. **Additive layer** — no Thorium backend modifications
5. **Agent decisions** — every response includes enough info for the next step
6. **Native terminology** — uses Thorium's concepts (Samples, Reactions, Images, Pipelines)

### Tool Inventory (18 total)

**Discovery tier (5 tools):**
| Tool | Status | Description |
|---|---|---|
| `list_groups` | New | Entry point — discover accessible groups |
| `list_pipelines` | Improved | Projected summaries with limit, has_more |
| `get_pipeline` | New | Stage ordering and image list |
| `list_images` | Improved | Projected summaries with limit, has_more |
| `get_image` | New | Dependencies, output format, used_by |

**Analyst tier (5 tools):**
| Tool | Status | Description |
|---|---|---|
| `get_sample` | Improved | SampleSummary projection, flattened tags |
| `get_sample_results` | Improved | Tool summary map (no content), filtering |
| `get_sample_result` | New | Scoped content fetch with max_chars truncation |
| `search_results` | New | Elasticsearch search, sha256 extraction, Kibana tag stripping |
| `start_tree` | Overhauled | Node projection, limit, groups, deterministic selection |

**Operator tier (5 tools):**
| Tool | Status | Description |
|---|---|---|
| `create_reaction` | New | Trigger pipeline on samples, optional per-image args |
| `get_reaction` | New | Poll status with polling guidance |
| `list_reactions` | New | Filter by status, has_more |
| `get_reaction_logs` | New | Stage logs for failure diagnosis |
| `cancel_reaction` | New (v1.1) | Delete/cancel a running or completed reaction |

**Documentation tier (3 tools, pre-existing):**
| Tool | Status | Description |
|---|---|---|
| `get_docs_toc` | Fixed | structuredContent wrapped in object |
| `get_doc_page` | Unchanged | Read specific doc page |
| `search_docs` | Fixed | structuredContent wrapped in object |

### Cross-Cutting Improvements

- SHA256 format validation on all sample-accepting tools
- MAX_LIST_LIMIT (500) ceiling and .max(1) floor on all list/search tools
- has_more pagination signal on all list tools
- content/structured_content shape alignment across all tools
- Fixed to_str().unwrap() panic on malformed auth headers
- truncate_json_value utility with 5 unit tests
- Search query length cap (1000 chars) and result ceiling (100 hits)
- Search hit field extraction from _source, highlight, and _id fallback
- Kibana highlight tag stripping from search excerpts

---

## Testing Results

### Automated Testing
- 49 unit tests passing (31 new + 18 pre-existing)
- `cargo +nightly build` compiles with zero errors and zero warnings in MCP code

### Code Review
- 6 independent code reviews with fresh-context agents
- End-to-end data flow verification across 6 agent workflows
- Adversarial edge case review

### Live Testing (Minithor)
- Deployed to local Minikube with Thorium 1.5.1
- 16 of 18 tools confirmed working against real data (2 need pipelines to test)
- End-to-end chain verified: search_results -> get_sample -> get_sample_results
- 1 tool (start_tree) blocked by pre-existing client library deserialization bug

### Bugs Found and Fixed During Testing
1. **Docs structuredContent** — bare arrays violated MCP spec (fixed: wrapped in objects)
2. **Search null fields** — tag index hits had null sha256/group (fixed: _id fallback parsing)
3. **Kibana highlight tags** — leaked into search excerpts (fixed: tag stripping)
4. **Stale credentials** — redeployment regenerates tokens (documented in setup guide)

---

## Files Changed

### New files
- `api/src/routes/mcp/groups.rs` — list_groups
- `api/src/routes/mcp/search.rs` — search_results
- `api/src/routes/mcp/reactions.rs` — create_reaction, get_reaction, list_reactions, get_reaction_logs

### Modified files
- `api/src/routes/mcp.rs` — module registration, auth panic fix
- `api/src/routes/mcp/files.rs` — get_sample projection, get_sample_results summary, get_sample_result, SHA256 validation, truncation utility
- `api/src/routes/mcp/images.rs` — field projection, get_image, limit caps
- `api/src/routes/mcp/pipelines.rs` — field projection, get_pipeline, limit caps, docstring fix
- `api/src/routes/mcp/trees.rs` — complete overhaul (node projection, limits, validation, relationship formatting)
- `api/src/routes/mcp/docs.rs` — structuredContent wrapped in objects

### Documentation
- `mcp-design-docs/thorium_mcp_architecture.md` — full design doc (updated to reflect implementation)
- `mcp-design-docs/adr-001-mcp-client-strategy.md` — ADR: MCP server first, defer custom TUI
- `mcp-design-docs/GETTING_STARTED_MCP.md` — setup guide for MCP clients (OpenCode, Cursor, etc.)
- `mcp-design-docs/MCP_SETUP_GUIDE.md` — local Minithor testing guide
- `mcp-design-docs/MCP_SERVER_TEST_REPORT.md` — live testing results
- `mcp-design-docs/NEXT_STEPS.md` — handoff document with deferred work
- `PR_DESCRIPTION.md` — ready-to-use PR description

### Configuration
- `minithor/thorium-cluster.yml` — added insecure_certificates for ES TLS in dev

---

## Known Limitations

1. **start_tree** — pre-existing client library deserialization bug (see BUG_REPORT_START_TREE.md)
2. **No grow_tree** — tree_id returned but no tool to expand trees (blocked by start_tree bug)
3. **No get_repo / get_entity** — tree nodes discoverable but no detail tools
4. **No cross-pipeline list_reactions** — requires pipeline parameter

---

## Commits (on feat/mcp-agent-tools)

1. `feat(mcp): Add agent-friendly discovery and analyst tools` — Sprint 1
2. `feat(mcp): Add reaction management tools for agent-driven analysis` — Sprint 2
3. `refactor(mcp): Harden tool surface and defuse start_tree context bomb` — Sprint 3
4. `fix(mcp): Wrap docs tool structuredContent in objects for MCP spec compliance`
5. `fix(mcp): Handle null fields in search hits and strip Kibana highlight tags`
6. `fix(mcp): Extract sha256 and group from ES document _id as fallback`
7. `feat(mcp): Add cancel_reaction and per-image args for create_reaction` — v1.1
