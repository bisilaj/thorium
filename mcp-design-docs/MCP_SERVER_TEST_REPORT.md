# Thorium MCP Server Test Report

**Date:** 2026-04-01 (round 3)
**Server target:** Dev server
**Group discovered:** `system`
**Sample discovered:** `mcp_test_sample.sh` (`a5e4fd...effddc36`)

---

## Summary

Tested all 17 MCP tools across three rounds. All original bugs are fixed except `start_tree`.

| Category | Tools | Count |
|----------|-------|-------|
| Working | list_groups, list_pipelines, list_images, get_sample, get_sample_results, get_sample_result, get_image, get_pipeline, list_reactions, get_docs_toc, search_docs, get_doc_page, search_results | 13 |
| Bug (response decoding) | start_tree | 1 |
| Not fully testable (no data) | get_reaction, get_reaction_logs, create_reaction | 3 |

---

## Fixed Since Initial Test

### 1. Docs tools: `structuredContent` schema mismatch -- FIXED

**Affected tools:** `get_docs_toc`, `search_docs`, `get_doc_page`

All three docs endpoints now work correctly:
- `get_docs_toc` returns a full TOC with 56 entries across intro, users, developers, admins,
  architecture, and help sections.
- `search_docs` returns ranked results with match counts and context snippets.
- `get_doc_page` returns full page content by path.

### 2. Elasticsearch TLS: self-signed certificate rejected -- FIXED

`search_results` now connects to Elasticsearch successfully and returns hits.

### 3. `search_results` null fields and Kibana tags -- FIXED

Previously, search hits had null `sha256` and `group` fields and raw Kibana highlighting
tags in excerpts. Now fixed:

**Before:**
```json
{"sha256": null, "group": null, "excerpt": "...@kibana-highlighted-field@system@/kibana-highlighted-field@..."}
```

**After:**
```json
{"sha256": "a5e4fd4ca60b2607ade584099cff3c1963c6b7dcdc22c21ac68c5411effddc36", "group": "system", "excerpt": "{\"group\":[\"system\"]}"}
```

sha256 and group are now populated, Kibana tags are stripped, and results are actionable for
downstream tool chaining (`search_results` -> `get_sample` -> `get_sample_results`).

---

## End-to-End Tool Chain Verified

Successfully chained: `search_results` -> `get_sample` -> `get_sample_results`

| Step | Tool | Result |
|------|------|--------|
| 1. Search | `search_results(query="*")` | Found `a5e4fd...` with group=system |
| 2. Get sample | `get_sample(sha256="a5e4fd...")` | `mcp_test_sample.sh`, uploaded 2026-04-01, tags: submitter=thorium |
| 3. Get results | `get_sample_results(sha256="a5e4fd...")` | 0 tools run (expected — no pipelines configured) |

---

## Remaining Issues

### 1. `start_tree`: response decoding failure

**Status:** Still broken across all three test rounds

**Error:**
```
error decoding response body
```

**Tested with:**
- Tag filter: `{"malware_family": ["emotet"]}` — fails
- Real sample: `a5e4fd4ca60b2607ade584099cff3c1963c6b7dcdc22c21ac68c5411effddc36` — fails

**Analysis:** The server receives a response from the Thorium API but cannot deserialize it.
This fails with both tag-based and sample-based queries, confirming it is a schema mismatch
between the MCP server's expected response type and what the API returns — not a data issue.
The Rust deserialization target likely does not match the actual response shape from the
tree/graph API endpoint.

**Severity:** Medium — tree traversal (relationship exploration) is non-functional.

---

## Tools Working Correctly

| Tool | Test | Result |
|------|------|--------|
| `list_groups` | No params | Returned `system` group |
| `list_pipelines` | group=system | Empty list (expected — no pipelines configured) |
| `list_images` | group=system | Empty list (expected — no images configured) |
| `get_pipeline` | nonexistent pipeline | Clean 'does not exist' error with trace ID |
| `get_image` | nonexistent image | Clean 'not found' error with trace ID |
| `list_reactions` | nonexistent pipeline | Clean 'does not exist' error with trace ID |
| `get_sample` | real sample a5e4fd... | Full metadata: name, hashes, tags, upload date |
| `get_sample_results` | real sample a5e4fd... | 0 tools run (correct) |
| `get_sample_result` | fake SHA256 + fake tool | Clean 'not found' error with trace ID |
| `search_results` | query=* in system | 1 hit with sha256 + group populated |
| `get_docs_toc` | No params | Full TOC, 56 entries |
| `search_docs` | query="pipeline" | 10 results with snippets |
| `get_doc_page` | path=intro.md | Full page content returned |
| `get_doc_page` | path=developers/build_pipelines.md | Full page content returned |

**Positive notes:**
- All "not found" errors include trace IDs for debugging
- Error messages are clear and descriptive
- `list_*` endpoints correctly return empty results rather than errors for an empty group
- Docs endpoints are comprehensive — full TOC, search, and page retrieval all work
- Search -> sample -> results chain works end-to-end

---

## Untested (blocked by missing data)

| Tool | Reason |
|------|--------|
| `get_reaction` | No reactions exist to query |
| `get_reaction_logs` | No reactions exist to query |
| `create_reaction` | No pipelines exist to run |

---

## Recommendations

1. **Fix `start_tree` response decoding** — log the raw response body from the API to identify
   the shape mismatch with the Rust deserialization target. This is the last remaining bug.
2. **Seed dev environment** — add at least one pipeline and image to enable end-to-end testing
   of create_reaction / get_reaction / get_reaction_logs and complete the full analysis
   workflow chain.
