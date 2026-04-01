# Getting Started: Using Thorium with an MCP Client

This guide shows how to connect an MCP-compatible AI assistant (OpenCode,
Cursor, etc.) to Thorium's MCP endpoint so you can interact with
Thorium conversationally.

---

## Prerequisites

- A running Thorium instance with the MCP endpoint enabled (API v1.5.1+)
- A valid Thorium API token (Bearer token)
- An MCP-compatible client installed

---

## 1. Get Your Thorium API Token

Log in to Thorium via the web UI or thorctl and obtain your API token:

```bash
thorctl login --host https://thorium.example.com --user your_username
```

Or retrieve an existing token from your Thorium settings page.

---

## 2. Configure Your MCP Client

### MCP Client Configuration

Add the following to your MCP client's configuration file (e.g., `.mcp.json`
in the project root):

```json
{
  "mcpServers": {
    "thorium": {
      "type": "http",
      "url": "https://thorium.example.com/mcp",
      "headers": {
        "Authorization": "Bearer YOUR_TOKEN_HERE"
      }
    }
  }
}
```

**Environment variable support:** Use `${VAR}` syntax to avoid hardcoding
tokens:

```json
{
  "mcpServers": {
    "thorium": {
      "type": "http",
      "url": "https://thorium.example.com/mcp",
      "headers": {
        "Authorization": "Bearer ${THORIUM_TOKEN}"
      }
    }
  }
}
```

### OpenCode

Add to `opencode.json` in your project root (or `~/.config/opencode/opencode.json`
for global config):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "thorium": {
      "type": "remote",
      "url": "https://thorium.example.com/mcp",
      "enabled": true,
      "headers": {
        "Authorization": "Bearer YOUR_TOKEN_HERE"
      }
    }
  }
}
```

**Environment variable support:** Use `{env:VAR}` syntax:

```json
{
  "headers": {
    "Authorization": "Bearer {env:THORIUM_TOKEN}"
  }
}
```

After configuring, run the `/mcps` command inside the OpenCode TUI to
verify the server loads correctly.

### Cursor

In Cursor settings, navigate to MCP Servers and add:

- **Name**: thorium
- **URL**: `https://thorium.example.com/mcp`
- **Headers**: `Authorization: Bearer YOUR_TOKEN_HERE`

---

## 3. Verify the Connection

Once configured, ask your AI assistant:

> "List the groups available in Thorium."

It should call `list_groups` and return the groups your token has access to.
If this works, you're connected.

---

## 4. Typical Workflows

### Explore what's available

```
"What groups do I have access to in Thorium?"
  -> calls list_groups

"What analysis pipelines are available in the malware-lab group?"
  -> calls list_pipelines(group="malware-lab")

"Tell me about the default-static pipeline."
  -> calls get_pipeline(group="malware-lab", pipeline="default-static")

"What does the capa image do?"
  -> calls get_image(group="malware-lab", image="capa")
```

### Analyze a sample

```
"Run the default-static pipeline on sample abc123...def in the malware-lab group."
  -> calls create_reaction, then polls get_reaction

"Is the reaction done yet?"
  -> calls get_reaction to check status

"Show me what tools have results for that sample."
  -> calls get_sample_results

"What did capa find?"
  -> calls get_sample_result(tool="capa")
```

### Search and investigate

```
"Search for any samples related to mimikatz in the incident-response group."
  -> calls search_results(query="mimikatz", groups=["incident-response"])

"Tell me about the first match."
  -> calls get_sample with the SHA256 from the search hit

"What analysis has been run on it?"
  -> calls get_sample_results
```

### Diagnose failures

```
"Are there any failed reactions for default-static in malware-lab?"
  -> calls list_reactions(status="Failed")

"What went wrong with that reaction?"
  -> calls get_reaction, get_pipeline, get_reaction_logs
```

### Explore relationships

```
"Find samples related to abc123...def."
  -> calls start_tree(samples=["abc123...def"])
  -> returns graph of related samples, repos, entities with relationship types

"Tell me more about the unpacked sample def789..."
  -> calls get_sample(sha256="def789...")
```

---

## 5. Available Tools (17 total)

### Discovery (start here)
| Tool | Purpose |
|---|---|
| `list_groups` | Discover accessible groups (call this first) |
| `list_pipelines` | List analysis pipelines in a group |
| `get_pipeline` | See pipeline structure (stages and images) |
| `list_images` | List analysis images (tools) in a group |
| `get_image` | See image details (dependencies, output format) |

### Analyst (explore data)
| Tool | Purpose |
|---|---|
| `get_sample` | Get sample metadata by SHA256 |
| `get_sample_results` | Summary of which tools have results (no content) |
| `get_sample_result` | Fetch specific tool's result content |
| `search_results` | Full-text search across results (Lucene syntax) |
| `start_tree` | Find related samples/repos/entities via graph traversal |

### Operator (run analysis)
| Tool | Purpose |
|---|---|
| `create_reaction` | Trigger a pipeline on samples |
| `get_reaction` | Poll reaction status and progress |
| `list_reactions` | List reactions, filter by status |
| `get_reaction_logs` | Get stage execution logs for diagnosis |

### Documentation
| Tool | Purpose |
|---|---|
| `get_docs_toc` | Browse documentation table of contents |
| `get_doc_page` | Read a specific documentation page |
| `search_docs` | Search documentation by keyword |

---

## 6. Tips

- **Start with `list_groups`** — group is required for most operations.
- **Use the two-step result pattern** — call `get_sample_results` first to
  see what tools ran, then `get_sample_result` for specific content.
- **Use `max_chars` for large results** — e.g.,
  `get_sample_result(sha256, tool, max_chars=3000)` to cap output size.
- **Diagnose failures with the 3-step pattern** — `get_reaction` to find
  the failed stage index, `get_pipeline` to map it to an image name,
  `get_reaction_logs` for the logs.
- **Poll reactions every 5-10 seconds** — reactions can take seconds to
  minutes depending on the pipeline and queue depth.

---

## 7. Troubleshooting

**"Missing authorization header" error**
- Verify your token is included in the MCP client configuration.
- Check that the Authorization header format is `Bearer YOUR_TOKEN`.

**"Invalid authorization header encoding" error**
- Your token may contain non-ASCII characters. Regenerate it.

**Empty results from list tools**
- Verify you have access to the group you're querying.
- Check that pipelines/images exist in that group.

**Search returns no hits**
- Elasticsearch indexing has a slight delay (~10 seconds).
- Verify the group name is correct.
- Try simpler queries first (single keyword).

**Reaction stays in "Created" status**
- The scaler may not have available workers. Check with an admin.
- The pipeline or its images may be banned. Try `get_pipeline` to check.
