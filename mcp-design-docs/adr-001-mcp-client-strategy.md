# ADR-001: MCP Server as Primary Agent Interface, Defer Custom TUI

**Status:** Accepted
**Date:** 2026-03-26
**Deciders:** Thorium MCP development team

---

## Context

Thorium needs an agent-friendly interface so that LLM agents can discover,
explore, and trigger analysis workflows. Three approaches were considered
for how analysts would interact with this interface:

1. **Custom Thorium TUI** — A dedicated terminal application that connects
   to the Thorium backend and uses an LLM to enable conversational analysis.
   A developer had been experimenting with this approach but encountered
   difficulty with tool calling integration and backend connection management.

2. **MCP server consumed by existing agent frameworks** — Expose Thorium's
   agent-optimized tools via the Model Context Protocol (MCP), allowing
   analysts to use any MCP-compatible client they already have (OpenCode,
   Cursor, Windsurf, etc.).

3. **Both** — Build the MCP server as the foundation and optionally wrap it
   in a custom TUI later.

## Decision

**Ship the MCP server first. Let analysts use it through existing agent
frameworks. Defer the custom TUI until real usage data justifies the
investment.**

If a custom TUI is built later, it should connect to the MCP server as a
thin client rather than integrating directly with the Thorium backend.

## Rationale

### The MCP server is the agent-friendly interface

The hard work is in the server layer: tool design, field projection, token
budgeting, the two-step result access pattern, reaction management. All of
that lives in the MCP server regardless of what client consumes it. A TUI
would call the same tools.

### Existing frameworks solve the hard client problems

Building a conversational TUI requires an agent runtime (tool calling, context
management, streaming, error recovery, multi-step workflows). MCP clients like
OpenCode and Cursor already solve these problems and are actively
maintained by the open source community. Building our own means duplicating
that work and taking on the maintenance burden.

### Adoption is faster with zero client code

An analyst already using an MCP client can connect to Thorium with a single
config entry:

```json
{
  "mcpServers": {
    "thorium": {
      "url": "https://thorium.internal/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

No installation, no new tool to learn. The barrier to adoption is near zero.

### Usage data should inform TUI design

Watching how analysts use Thorium through existing MCP clients reveals which
workflows are common, what is missing, and what is awkward. Building the TUI
first means guessing at these answers. Building the MCP server first lets us
observe and then design a TUI informed by real usage patterns.

### The prior TUI experiment validates this approach

The difficulty encountered with the TUI prototype — tool calling integration,
backend connections — is exactly the kind of complexity that MCP abstracts
away. The protocol handles transport, session management, and tool dispatch.
A future TUI built as a thin MCP client would sidestep most of those issues.

## Consequences

### Positive

- Faster time to value: analysts can use the MCP tools as soon as the server
  is deployed, without waiting for a custom client.
- Lower maintenance burden: no client code to build, test, or update when
  tools change.
- Broader compatibility: works with any MCP-compatible client, including
  future ones we cannot predict today.
- The MCP server becomes a stable API contract that multiple clients can
  consume.

### Negative

- Less control over the analyst experience: we cannot customize how results
  are displayed or add Thorium-specific UI elements (progress visualizations,
  result viewers, etc.).
- Dependent on external frameworks: if an analyst does not already use an
  MCP-compatible tool, they must adopt one.
- Cannot embed in Thorium's existing web UI without additional work.

### If we revisit

A custom TUI becomes worthwhile when:

- There are 2-3 well-understood analyst workflows that benefit from
  purpose-built UI (e.g., triage dashboards, incident timelines).
- The MCP tool surface is stable enough that the TUI is not constantly
  chasing API changes.
- There is developer capacity to maintain both the MCP server and the client.

When that time comes, the TUI should be a thin MCP client using a library
like `rmcp`, not a direct Thorium backend integration. The tool surface,
response shapes, and authentication model are already defined by the MCP
server.

## Related

- `mcp-design-docs/thorium_mcp_architecture.md` — MCP server architecture and
  tool surface design.
