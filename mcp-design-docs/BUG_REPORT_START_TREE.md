# Bug Report: start_tree Response Deserialization Failure

**Component:** Thorium Rust client library (`crate::client::Trees::start`)
**Severity:** Medium
**Affects:** MCP `start_tree` tool (and any code using `Trees::start()` via loopback)
**Does NOT affect:** Direct REST API calls to `POST /api/trees/`

---

## Summary

The `Trees::start()` method in the Thorium client library fails to
deserialize the response from `POST /api/trees/` with "error decoding
response body", even though the REST endpoint returns valid JSON.

---

## Reproduction Steps

### 1. Via MCP (fails)

Connect to the MCP endpoint and call `start_tree`:

```json
{
  "method": "tools/call",
  "params": {
    "name": "start_tree",
    "arguments": {
      "samples": ["a5e4fd4ca60b2607ade584099cff3c1963c6b7dcdc22c21ac68c5411effddc36"]
    }
  }
}
```

**Result:** `{"error": {"code": -32603, "message": "error decoding response body"}}`

### 2. Via REST API directly (works)

```bash
TOKEN="<valid_token>"
ENCODED=$(echo -n "$TOKEN" | base64)
curl -s -X POST "http://localhost/api/trees/?limit=50" \
  -H "Authorization: token $ENCODED" \
  -H "Content-Type: application/json" \
  -d '{"samples":["a5e4fd4ca60b2607ade584099cff3c1963c6b7dcdc22c21ac68c5411effddc36"]}'
```

**Result:** Valid JSON response with tree data (see sample output below).

### 3. Via Thorium client library (fails)

Any code using the client library's tree method:

```rust
let tree = thorium.trees.start(&TreeOpts::default(), &query).await?;
// Error: "error decoding response body"
```

---

## Root Cause Analysis

The client library's `Trees::start()` method calls `send_build!(self.client,
req, Tree)` which does `response.json::<Tree>()` — strict serde
deserialization. The response JSON contains nested types that don't
round-trip cleanly through serde.

The `Tree` struct contains `data_map: HashMap<u64, TreeNode>` where each
`TreeNode` is one of:
- `TreeNode::Sample(Sample)` — full Sample with tags, submissions, comments
- `TreeNode::Repo(Repo)` — full Repo with tags, submissions
- `TreeNode::Entity(Entity)` — full Entity with metadata
- `TreeNode::Tag(TreeTags)` — tag filter

The deserialization fails somewhere in these deeply nested types. The most
likely candidates are:

### Candidate 1: `Origin` enum in `SubmissionChunk`

The API serializes `Origin::None` as the string `"None"` in JSON:

```json
"origin": "None"
```

Serde should handle this for a unit variant, but if the `Origin` enum has
`#[serde(tag = "...")]` or other serialization attributes that change the
expected format, this could fail.

### Candidate 2: `TagMap` type

`TagMap` is `HashMap<String, HashMap<String, HashSet<String>>>`. The API
response shows:

```json
"tags": {
  "submitter": {
    "thorium": ["system"]
  }
}
```

If `HashSet<String>` deserializes differently than a JSON array, or if the
inner HashMap has issues with certain key types, this could fail silently.

### Candidate 3: `Comment` or `Attachment` types

The `Sample` struct includes `comments: Vec<Comment>` which may contain
nested types (attachments, etc.) that have deserialization mismatches.

---

## How to Diagnose

The fastest way to find the exact failing field:

### Option A: Add response body logging

In `api/src/client/trees.rs`, temporarily replace:

```rust
send_build!(self.client, req, Tree)
```

With:

```rust
let resp = self.client.post(&url)
    .header("authorization", &self.token)
    .json(query)
    .query(&query_params)
    .send()
    .await?;
let body = resp.text().await?;
tracing::error!("Tree response body: {}", &body[..500.min(body.len())]);
let tree: Tree = serde_json::from_str(&body)?;
Ok(tree)
```

This will log the raw response body, then try to deserialize it. The
`serde_json::from_str` error will include the exact byte position and
field where deserialization fails.

### Option B: Unit test

Write a test that serializes a `Tree` struct to JSON, then deserializes it
back. Start removing fields until the round-trip succeeds to isolate the
problem field:

```rust
#[test]
fn tree_roundtrip() {
    let json = r#"<paste raw API response here>"#;
    let result: Result<Tree, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Failed: {:?}", result.err());
}
```

---

## Sample API Response (for reproduction)

This is the actual JSON returned by `POST /api/trees/?limit=50` for a test
sample on a Minithor instance:

```json
{
  "id": "fe134392-ce3a-480c-aa1a-4721a7dd3991",
  "initial": [18274003433511019150],
  "growable": [],
  "data_map": {
    "18274003433511019150": {
      "Sample": {
        "sha256": "a5e4fd4ca60b2607ade584099cff3c1963c6b7dcdc22c21ac68c5411effddc36",
        "sha1": "63af8ae2edac40fcde51ae31dd28a27af7f6dce7",
        "md5": "a23ef5d9534f99c309a6afffcba62760",
        "tags": {
          "submitter": {
            "thorium": ["system"]
          }
        },
        "submissions": [
          {
            "id": "36522952-b5ba-4852-b534-312ec4165282",
            "name": "mcp_test_sample.sh",
            "description": null,
            "groups": ["system"],
            "submitter": "thorium",
            "uploaded": "2026-04-01T13:48:03.723Z",
            "origin": "None"
          }
        ],
        "comments": []
      }
    }
  },
  "branches": {}
}
```

Note the `"origin": "None"` field — this is the most likely deserialization
failure point.

---

## Impact

- The MCP `start_tree` tool is non-functional
- Any code using `thorium.trees.start()` via the client library may be affected
- The REST API endpoint works correctly — only the client-side deserialization fails
- Direct REST API consumers (web UI, curl) are NOT affected

---

## Suggested Fix

Once the failing field is identified via the diagnosis steps above, the fix
is likely one of:

1. **Add `#[serde(deserialize_with = "...")]`** on the problematic field to
   handle the format mismatch
2. **Add `#[serde(default)]`** if the field is sometimes missing
3. **Fix the serialization** on the API side to match what the struct expects
4. **Add `#[serde(untagged)]`** or adjust enum representation if an enum
   variant serializes differently than expected

The fix would be in the models crate (`api/src/models/`), not in the MCP
layer or the client library.

---

## Workaround

There is no workaround in the MCP layer since the error occurs in the
Thorium client library before the MCP handler receives the data. The MCP
`start_tree` tool correctly propagates the error to the agent.

Agents can use `search_results` as an alternative for discovering related
samples — it provides text-based discovery rather than graph traversal.
