# Minithor MCP Testing Setup Guide

This guide documents the local Minithor setup for testing the MCP agent tools.

**Last updated:** 2026-04-01
**Environment:** macOS ARM64 (M1/M2), Docker Desktop, Minikube

---

## Current Local State

**Minikube cluster:** Running (4 CPUs, 7GB RAM)
**Thorium version:** 1.5.1 (built from `feat/mcp-agent-tools` branch)
**Container image:** `ghcr.io/bisilaj/thorium/infrastructure/thorium:main`
**MCP endpoint:** `http://localhost/api/mcp`
**Admin user:** `thorium` (password changes on each deployment — see "Getting a fresh auth token" below)

**Note:** Credentials are regenerated every time the ThoriumCluster resource is
recreated. Always get a fresh token after redeployment.

---

## Starting Minithor (after a reboot or shutdown)

If Minikube has been stopped, restart it:

```bash
# Start minikube
minikube start --cni calico

# Verify all pods are running
minikube kubectl -- get pods --all-namespaces | grep -E "thorium|redis|scylla|elastic|minio"

# Start the tunnel (run in a dedicated terminal, will prompt for password)
minikube tunnel
```

If the Thorium pods are not running (e.g., after a fresh minikube start):

```bash
# Check if the ThoriumCluster resource exists
minikube kubectl -- get thoriumclusters -n thorium

# If not, deploy it
minikube kubectl -- create -n thorium -f minithor/thorium-cluster.yml
```

## Getting a fresh auth token

Tokens expire after ~90 days. To get a new one:

```bash
# Get the thorium user password
minikube kubectl -- get secret -n thorium thorium-pass \
  --template='{{.data.thorium}}' | base64 --decode; echo

# Authenticate (replace PASSWORD with the output above)
BASIC=$(echo -n "thorium:PASSWORD" | base64)
curl -s -X POST -H "Authorization: Basic $BASIC" \
  http://localhost/api/users/auth
```

This returns a JSON object with `token` and `expires` fields.

## MCP Client Configuration

### Project-scoped setup (recommended)

Create or edit `.mcp.json` in the project root with the following configuration:

```json
{
  "mcpServers": {
    "thorium": {
      "type": "http",
      "url": "http://localhost/api/mcp",
      "headers": {
        "Authorization": "Bearer YOUR_TOKEN_HERE"
      }
    }
  }
}
```

To use environment variables, set `${THORIUM_TOKEN}` in the configuration:

```json
{
  "mcpServers": {
    "thorium": {
      "type": "http",
      "url": "http://localhost/api/mcp",
      "headers": {
        "Authorization": "Bearer ${THORIUM_TOKEN}"
      }
    }
  }
}
```

Then set the env var before starting your MCP client:

```bash
export THORIUM_TOKEN="d563f377e91ccccfe269e4ee0861f953227050f9bd1b7b835b20419888899122"
```

## Verifying the MCP connection

In a new MCP client session, ask:

```
"List the groups I have access to in Thorium."
```

Expected response: the `system` group (the only group in a fresh Minithor).

## Uploading test data

A fresh Minithor has no samples, pipelines, or images. To test the full
workflow, you need to:

1. **Create a group** (the `system` group exists but may need pipelines)
2. **Upload a sample** (a binary file for analysis)
3. **Create images** (Docker analysis tools — requires the tool containers)
4. **Create a pipeline** (defines which images to run)

For basic MCP testing without analysis tools, you can upload a sample and
verify the discovery + result tools work with whatever results are available.

### Upload a sample via the REST API

```bash
# Upload a file as a sample
curl -X POST http://localhost/api/samples/ \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -F "file=@/path/to/test/file" \
  -F "groups=[\"system\"]"
```

### Verify via MCP

After uploading, test the MCP tools:

```
"Get info about sample <sha256>"
"What tool results exist for sample <sha256>?"
```

## Stopping Minithor

```bash
# Stop minikube (preserves state)
minikube stop

# Or delete everything (destructive)
minikube delete
```

## Rebuilding after code changes

If you modify the MCP tools and need to redeploy:

1. Commit and push to `main` on your fork
2. Wait for GitHub Actions to build the container (~25 min)
3. Delete and recreate the Thorium deployment:

```bash
minikube kubectl -- delete thoriumclusters -n thorium dev
# Wait for pods to terminate
minikube kubectl -- get pods -n thorium
# Redeploy
minikube kubectl -- create -n thorium -f minithor/thorium-cluster.yml
```

## Troubleshooting

**MCP returns "Missing authorization header"**
- Verify the token is in the `.mcp.json` configuration
- Restart your MCP client after changing MCP config

**MCP returns 406 Not Acceptable**
- The MCP client must send `Accept: application/json, text/event-stream`
- This is handled automatically by most MCP clients; only affects manual curl testing

**Pods stuck in ImagePullBackOff**
- The container image may not be accessible
- Check: `minikube kubectl -- describe pod -n thorium <pod-name>`
- Verify the image exists: `docker pull ghcr.io/bisilaj/thorium/infrastructure/thorium:main`

**Tunnel not working**
- `minikube tunnel` must be running in a separate terminal
- It requires sudo/password for privileged port forwarding

**Token expired**
- Re-authenticate using the basic auth flow above
- Update `.mcp.json` with the new token
- Restart your MCP client
