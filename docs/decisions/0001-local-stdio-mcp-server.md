# 0001. A local stdio MCP server; the agent owns the model

**Date:** 2026-09-28 · **Status:** Accepted

## Context

Engineers investigating hardware behaviour already work with AI agents (Claude Code, Codex, Cursor, Cline…). These agents bring broad technical knowledge, a model, the engineer's own API key and token budget. What they lack is access to the engineer's recordings and a memory that outlives one conversation.

## Decision

Wise Bucket is a local **Model Context Protocol (MCP) server** over stdio, written in Rust. Each agent window starts it as a subprocess. The agent owns the model, the keys, the budget and the conversation; **Wise Bucket never calls a model**. It only reads folders the engineer declares (see [0004](0004-user-wide-only.md)).

## Alternatives

- **A web service or a hosted platform:** recordings are large and often confidential; moving them off the machine is the problem we avoid.
- **An agent with its own model calls:** it would duplicate what every harness already does, and take over keys and costs that belong to the engineer.

## Consequences

- Works with any MCP-capable agent, with no account and no network.
- Each agent window runs its own process: anything shared between windows must be safe for several processes ([0011](0011-no-database.md)).
- Wise Bucket can only answer when called: it cannot start a conversation.
