# 0009. Hardware knowledge deferred, with its open problems

**Date:** 2026-10-06 · **Status:** Accepted

## Context

Milestone M1 was first planned as **hardware knowledge**: entities (sensors, buses, software), facts with provenance, aliases, and a resolver that matches what the engineer says ("the Ouster", "lidar avant") to known parts. Designing it raised problems without good answers yet.

## Decision

Hardware knowledge is **deferred** and listed as a possible feature ([0014](0014-possible-features-instead-of-milestones.md)), with its open problems:

- **Overlap with the agent's own memory:** `CLAUDE.md`, auto-memory and memory banks are loaded at the start of every session, while Wise Bucket answers only when asked. A stale note in the agent's memory silently wins, and MCP gives no way to coordinate the two.
- **Who matches names:** a fixed similarity threshold on the server judges worse than the LLM. The direction taken since ([0010](0010-metrics-are-sql-written-by-the-agent.md)) is for the agent to query with native text matching, and to judge.
- **Verifying what the engineer said:** the server cannot tell whether a fact recorded as "stated by the engineer" really was. Verbatim quotes, "unconfirmed" flags, per-call approval or MCP elicitation are options.
- **Corrections over time:** superseding facts, validity windows (configuration changes), stale and conflicting facts.
- **Whether a seed ontology is needed at all,** given the LLM's own domain knowledge.
- **What predictions refer to** in a future investigation feature, without entities: streams and columns are the candidates.

## Alternatives

- **Building the knowledge base in M1:** set aside to deliver access to the data first.

## Consequences

- M1 became the columnar store and SQL ([0011](0011-no-database.md), [0012](0012-trade-storage-for-query-speed.md)).
