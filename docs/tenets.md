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

An ADR explains a past choice; a new design is judged on these tenets and may reverse any ADR.

*Gives up:* past decisions as an argument. · [0011], [0021], [0025]

[0010]: decisions/0010-metrics-are-sql-written-by-the-agent.md
[0011]: decisions/0011-no-database.md
[0012]: decisions/0012-trade-storage-for-query-speed.md
[0017]: decisions/0017-result-size-and-paging.md
[0021]: decisions/0021-parquet-for-streams.md
[0022]: decisions/0022-unreadable-formats-are-listed.md
[0024]: decisions/0024-recording-context-from-the-files.md
[0025]: decisions/0025-context-from-the-engineer.md
[0026]: decisions/0026-column-types-and-ulog-formats.md
