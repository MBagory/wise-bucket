# 0023. Quiet server logs

**Date:** 2026-10-13 · **Status:** Accepted

## Context

The MCP server wrote every log line to `server.log` and to stderr. Claude Code keeps each stderr line of an MCP server in its own MCP log, marked as an error. About half of the lines came from the `rmcp` crate: its handshake at INFO (up to 636 characters, with the client's capabilities), and a WARN for each error response it sent the client. While disk space was low, `extraction paused` was written at every pass: 47 times in 2.5 hours on 2026-10-12. A failed extraction copied the child's whole error (message, code, fix and docs) into the log.

## Decision

- `server.log` keeps INFO and above, with `rmcp` at ERROR. `WB_LOG` still replaces the filter.
- The MCP server writes only WARN and above to stderr.
- `extraction paused` is logged when the pause starts, and `extraction resumed` when it ends.
- An extraction line carries the child's message only: no `✔`/`✖` mark, no second copy of the recording, no code, fix and docs lines. The full error stays in the `error` column of `recordings`.

## Alternatives

- **Nothing on stderr:** warnings would no longer reach the agent's MCP log, the first place an engineer looks when the server does not connect.
- **`rmcp` at WARN:** keeps one warning per session for an error response the client already receives.

## Consequences

- A session logs its start, its client, one line per extraction, and its end.
- `rmcp`'s handshake and error responses are logged only with `WB_LOG` (for example `WB_LOG=info,rmcp=debug`).
