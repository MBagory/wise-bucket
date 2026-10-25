# Engineering tenets

Ranked: when two conflict, the higher one wins. Changing a tenet takes a new [decision record](decisions/README.md).

## 1. Frugal and efficient with the agent's context

Return the least data that answers, in the fewest calls.

*Gives up:* future-proofing and backward compatibility. · [0017], [0024], [0026]

## 2. Context stays attached to the data

What the files and the engineer say about a recording sits where the agent already looks, and a missing piece is stated.

*Gives up:* slower extraction, a longer first result. · [0022], [0024], [0025]

## 3. Decode once, query fast

Every recording is stored again as decoded data, never as precomputed answers; the agent writes the SQL.

*Gives up:* disk space. · [0010], [0012], [0021]

## 4. Decision records are history, not precedent

An ADR may be edited until the release that ships it, then never again: a released ADR is a historical record. New designs are judged on these tenets, and a new ADR may update or reverse any earlier one.

*Gives up:* past decisions as an argument, and fixing a released record in place. · [0011], [0021], [0025]

## 5. Spike findings are versioned

A spike's findings are written in Markdown in [`spikes/`](spikes/) and committed with the change they inform.

*Gives up:* throwaway local notes. · [0003]

[0003]: decisions/0003-managed-postgresql.md
[0010]: decisions/0010-metrics-are-sql-written-by-the-agent.md
[0011]: decisions/0011-no-database.md
[0012]: decisions/0012-trade-storage-for-query-speed.md
[0017]: decisions/0017-result-size-and-paging.md
[0021]: decisions/0021-parquet-for-streams.md
[0022]: decisions/0022-unreadable-formats-are-listed.md
[0024]: decisions/0024-recording-context-from-the-files.md
[0025]: decisions/0025-context-from-the-engineer.md
[0026]: decisions/0026-column-types-and-ulog-formats.md
