# OpenAI Compatibility Strategy

pig is a distributed inference platform and control plane, not an agent
framework. Agent harnesses such as Goose and Codex own planning, tool execution,
repository interaction, and retry loops. pig owns authentication, routing,
scheduling, benchmarking, metrics, worker lifecycle, and protocol compatibility.

## Gateway

The coordinator provides a deliberately thin OpenAI-compatible boundary:

- `GET /v1/models`
- `POST /v1/chat/completions`

It translates request and response shapes only. It must never grow a second
scheduler, routing policy, worker selection mechanism, benchmark store, metrics
system, or authentication layer.

```text
OpenAI client → coordinator gateway → ModelRequest → scheduler → worker → ModelResponse
```

`/v1/models` publishes stable logical policies (`pig-coding`,
`pig-reasoning`, and `pig-verification`). Physical model identifiers are
operator-facing information and are not part of the client contract.

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

## Integration boundary

Goose or another harness runs on the user's workstation with its MCP tools and
workspace access. It authenticates only to the coordinator. Worker addresses,
worker credentials, selected physical models, and backend details remain inside
pig's control plane.
