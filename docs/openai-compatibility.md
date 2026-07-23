# OpenAI Compatibility Strategy

pig is a distributed inference platform and control plane, not an agent
framework. Agent harnesses such as Goose and Codex own planning, tool execution,
repository interaction, and retry loops. pig owns authentication, routing,
scheduling, benchmarking, metrics, worker lifecycle, and protocol compatibility.

## Gateway

The coordinator provides a deliberately thin OpenAI-compatible boundary:

- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/pipeline`

It translates request and response shapes only. It must never grow a second
scheduler, routing policy, worker selection mechanism, benchmark store, metrics
system, or authentication layer.

```text
OpenAI client → coordinator gateway → ModelRequest → scheduler → worker → ModelResponse
```

`/v1/models` returns two kinds of entries: the stable logical role aliases
(`pig-coding`, `pig-reasoning`, `pig-verification`) and the actual model IDs
registered in the coordinator's `pig.toml`. Clients that want role-based routing
use the aliases; clients that need a specific physical model pass its ID directly.

## Tool calls and streaming

Tool definitions, `tool_choice`, and backend-generated `tool_calls` remain
structured. A request requiring tools is rejected when the selected worker does
not advertise tool support; pig must not silently discard tool semantics.

Conversation history is also structured: assistant tool calls retain their IDs,
function names, and argument text, while tool-result messages retain their
`tool_call_id` links and original ordering. These protocol-neutral records flow
through `ModelRequest` without becoming prompt text, so scheduling remains
unaware of OpenAI-specific message shapes.

Streaming follows the same boundary. Workers emit canonical `ModelChunk` values
(`text_delta`, `tool_call_delta`, and `finished`); the coordinator relays them
without buffering, and only the gateway translates them into OpenAI SSE chunks.
Dropping a downstream stream closes the worker response body, so the existing
bounded worker channel applies backpressure instead of accumulating a response.

## Pipeline

`POST /v1/pipeline` executes a sequence of model invocations in order, each
dispatched through the full scheduler.

```json
{
  "session_affinity": true,
  "steps": [
    {
      "role": "reasoning",
      "messages": [{"role": "user", "content": "Draft an answer to: ..."}],
      "inject_previous": false
    },
    {
      "role": "verification",
      "messages": [{"role": "user", "content": "Is the above accurate?"}],
      "requirements": {"preferred_accelerator": "cuda"},
      "inject_previous": true
    }
  ]
}
```

`role` defaults to `reasoning` when omitted. `requirements` is the same
`ModelRequirements` struct used by `/v1/generate` — accelerator preference,
memory floor, placement policy, capability constraints, timeouts:

```json
{
  "minimum_context_tokens": 32768,
  "require_accelerator": true,
  "reasoning": true,
  "tool_calling": true
}
```

`reasoning: true` hard-rejects any model not tagged `reasoning = true` in the
registry. `tool_calling: true` is enforced the same way. When `inject_previous` is `true`, the
previous step's text output is prepended as an `assistant` message before the
current step's messages; the caller controls context threading, pig executes it.

`session_affinity` (default `false`): when `true`, all steps after the first are
pinned to the worker that served step 0. This keeps a multi-step computation on
the same warm machine — useful when each step builds on the same model's context
window. If the pinned worker becomes unavailable mid-pipeline, the step fails
rather than silently migrating to a different worker.

Response:

```json
{
  "steps": [
    {"step": 0, "content": "..."},
    {"step": 1, "content": "..."}
  ]
}
```

A failed step returns an HTTP error with the step index in the message; prior
steps' outputs are not returned on partial failure.

### Routing response headers

The `POST /v1/generate` endpoint includes these headers on every response so
callers can inspect routing decisions without a separate `POST /v1/route` call:

| Header | Example | Meaning |
|---|---|---|
| `X-Pig-Worker-Id` | `m4-worker` | Worker that served this request |
| `X-Pig-Model-Id` | `qwen3-8b-q4` | Physical model that was used |

## Integration boundary

Goose or another harness runs on the user's workstation with its MCP tools and
workspace access. It authenticates only to the coordinator. Worker addresses,
worker credentials, selected physical models, and backend details remain inside
pig's control plane.
