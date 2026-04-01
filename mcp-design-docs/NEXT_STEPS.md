# MCP Agent Tools — Next Steps

**Last updated:** 2026-04-01
**Current state:** Sprints 1-3 + v1.1 complete. Locally tested on Minithor.
Waiting on devs for upstream merge and staging deployment with real data.

---

## What's ready now

### Code (merged to `main` on personal fork)

18 MCP tools implemented across 8 module files, 49 unit tests passing:
- `mcp/groups.rs` — list_groups
- `mcp/pipelines.rs` — list_pipelines, get_pipeline
- `mcp/images.rs` — list_images, get_image
- `mcp/files.rs` — get_sample, get_sample_results, get_sample_result
- `mcp/search.rs` — search_results
- `mcp/reactions.rs` — create_reaction (with args), get_reaction, list_reactions,
  get_reaction_logs, cancel_reaction
- `mcp/trees.rs` — start_tree (overhauled)
- `mcp/docs.rs` — get_docs_toc, get_doc_page, search_docs

Build: `cargo +nightly build -p thorium-api --lib`
Test: `cargo +nightly test -p thorium-api --lib -- mcp` (49 tests)

### Documentation (in `mcp-design-docs/`)

| File | Purpose |
|---|---|
| `thorium_mcp_architecture.md` | Full architecture design doc |
| `adr-001-mcp-client-strategy.md` | ADR: MCP server first, defer custom TUI |
| `MCP_IMPLEMENTATION_SUMMARY.md` | Complete summary of all work done |
| `BUG_REPORT_START_TREE.md` | Detailed bug report for devs |
| `MCP_SERVER_TEST_REPORT.md` | Live testing results (3 rounds) |
| `GETTING_STARTED_MCP.md` | Setup guide for analysts (MCP clients such as OpenCode, Cursor, etc.) |
| `MCP_SETUP_GUIDE.md` | Local Minithor testing guide |
| `NEXT_STEPS.md` | This file |
| `PR_DESCRIPTION.md` | Ready-to-use PR description |

### Live testing status

16 of 18 tools confirmed working on local Minithor:
- 13 fully tested with real data
- 3 working but need pipelines/images to fully test (create_reaction,
  get_reaction, get_reaction_logs)
- 1 working but untested locally (cancel_reaction — needs a reaction to cancel)
- 1 blocked by pre-existing bug (start_tree — see BUG_REPORT_START_TREE.md)

End-to-end chain verified: search_results -> get_sample -> get_sample_results

---

## What's needed from the devs

1. **Review and fix `start_tree` deserialization bug** — see
   `BUG_REPORT_START_TREE.md` for full reproduction steps and diagnosis guide.
   This is a pre-existing issue in the Thorium client library, not in MCP code.

2. **Merge upstream `feat/mcp-docs-tools` branch** — our branch depends on it.
   Once merged, rebase `feat/mcp-agent-tools` onto main and open MR.

3. **Deploy to staging with real data** — pipelines, images, and samples are
   needed to test create_reaction -> get_reaction -> get_sample_results
   end-to-end.

---

## When the upstream branch (`feat/mcp-docs-tools`) merges

1. **Rebase onto main:**
   ```bash
   git fetch origin
   git rebase origin/main
   ```

2. **Verify the build still works:**
   ```bash
   cargo +nightly build -p thorium-api --lib
   cargo +nightly test -p thorium-api --lib -- mcp
   ```

3. **Open a new MR** from `feat/mcp-agent-tools` targeting `main`. Use the
   content from `PR_DESCRIPTION.md`.

---

## When the code is deployed to staging

### Initial smoke test

1. Verify the MCP endpoint is accessible:
   ```bash
   curl -H "Authorization: Bearer <token>" https://staging.thorium.example/api/mcp
   ```
   Note: MCP endpoint is at `/api/mcp`, not `/mcp`.

2. Configure your MCP client using `GETTING_STARTED_MCP.md`.

3. Run through the golden-path workflow conversationally:
   ```
   "List the groups I have access to."
   "What pipelines are in <group>?"
   "Tell me about the <pipeline> pipeline."
   "What does the <image> image do?"
   "Run <pipeline> on sample <sha256> with args {'<image>': {'switches': ['--verbose']}}."
   "Is the reaction done?"
   "What results came back?"
   "Show me what <tool> found."
   ```

4. Run through the failure diagnosis workflow:
   ```
   "Are there any failed reactions for <pipeline> in <group>?"
   "What went wrong with <reaction_id>?"
   "Cancel that reaction."
   ```

5. Test search:
   ```
   "Search for samples matching <keyword> in <group>."
   ```

6. Test tree traversal (after start_tree bug is fixed):
   ```
   "Find samples related to <sha256>."
   ```

### What to watch for

- **Auth errors** — Credentials regenerate on each ThoriumCluster redeployment.
  Get a fresh token via basic auth to `/api/users/auth`.
- **Empty results** — If lists return empty, verify the group has pipelines/
  images/data. The `has_more` flag should be false for genuinely empty results.
- **Large responses** — Check `limit` and `max_chars` parameters. Defaults are
  conservative (25 for trees, 100 for lists, 20 for search).
- **Reaction args** — Args are optional. If provided, they must be a JSON object
  mapping image names to `{positionals, kwargs, switches}`. Invalid format
  returns a clear error message.
- **Search excerpts** — Kibana highlight tags should be stripped. If you see
  `@kibana-highlighted-field@` markers, the fix didn't deploy correctly.

---

## Known deferred work

### Blocked on start_tree fix

| Item | Notes |
|---|---|
| `grow_tree` tool | Backend `client.trees.grow()` exists, needs MCP wrapper |
| `get_repo` / `get_entity` detail tools | Tree discovers these nodes but can't drill in |

### Future enhancements

| Item | Notes |
|---|---|
| Cross-pipeline `list_reactions` | Requires pipeline param currently; could add backend route |
| Synthesis tier (`correlate_results`, `diagnose_reaction`, `summarize_results`) | Needs inner LLM infrastructure |
| Semantic search | Needs embedding backend for `search_results` |

### Pre-existing issues (not introduced by this work)

| Item | Notes |
|---|---|
| `search_docs` reads all files on every call | Could benefit from caching |
| `search_docs` unbounded params | `max_results`, `max_snippets`, `context_lines` have no ceiling caps |
| Docs tools only check token presence, not validity | `grab_token` vs `client()` auth pattern |

---

## Key files to re-read when resuming

1. **This file** — overall status and plan
2. **`MCP_IMPLEMENTATION_SUMMARY.md`** — complete summary of all work
3. **`thorium_mcp_architecture.md`** Section 5 — tool surface spec
4. **`MCP_SERVER_TEST_REPORT.md`** — live testing results
5. **`api/src/routes/mcp.rs`** — router registration and module structure
