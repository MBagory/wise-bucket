# 0025. Context from the engineer: a `context` column and a `note` tool

**Date:** 2026-10-13 · **Status:** Accepted (supersedes [0008](0008-hardware-rather-than-robot.md) for the root option; the vocabulary stays)

## Context

Some facts about a recording are in no file: which machine produced it, what changed before the test, the conditions, the purpose of the test. The only place for them was the root option `--hardware` ([0008](0008-hardware-rather-than-robot.md)): one label for every recording of a folder, empty in practice, and no place for the other facts.

[0024](0024-recording-context-from-the-files.md) made the files' own context visible to the agent. The design criterion is the same here: the agent must find this context where it already looks, and record it in as few calls as possible.

## Decision

- **Read:** `recordings.context`, a map of key → value per recording, in the table the agent queries first. `context['hardware'] = 'drone-2'` filters on it; results show it as a JSON object.
- **Write:** a `note` tool, `note(target, context)`, and the command `wisebucket note <target> key=value…`. The target is a recording (`root:path`), a folder (`root:folder/`) or a root (`root:`); a folder named without its `/` is still that folder. Several keys go in one call; an empty value removes a key; no keys returns the target's current ones. An unknown target is a `target_not_found` error with a fix.
- **Resolution:** for each key, the most specific target wins: the recording, then its folders, then its root.
- **Storage:** `context.toml`, next to the user configuration: one table per target, editable by hand, written atomically under a file lock. An invalid file fails queries with `context_invalid`, which names the file.
- **`hardware`** is a key like the others, documented as the machine. The root option `--hardware`, its configuration key, the `setup` question and the `hardware` column are removed. The word "hardware" stays as [0008](0008-hardware-rather-than-robot.md) chose it.

## Alternatives

- **Notes files in the data folders** (`wisebucket.toml` per folder, inherited by subfolders, as BIDS sidecars are): shared with the data, but the agent must find the folder, read and edit TOML (two or three calls, with syntax to get right), they need write access to the data, and the agent learns of them only through the instructions.
- **Key/values per root in the configuration** (`--property vehicle=drone-2`): one set per folder, nothing per recording, and in a file the agent does not read.
- **A separate `context` table** (`recording`, `key`, `value`): the agent would have to join it, while a map column shows in the first query.
- **Keeping the `hardware` column next to the map:** two places for the machine.
- **The agent's own memory:** not queryable with SQL, not tied to recordings, not seen by other agents.

## Consequences

- The agent sees what the engineer said in its first query, and records several facts in one call.
- `note` is Wise Bucket's first tool that writes; it writes only `context.toml`, never the recordings or the roots. Harnesses that ask before a tool call show the engineer what is recorded.
- The context stays on this machine, next to the settings; teammates do not see it unless the file is shared.
- A configuration that still has `hardware = …` in a root fails with `config_invalid`, which names the key.
- A renamed or moved recording loses the context of its old path; folder and root targets keep applying.
- Time ranges ("gust from 154 to 158 s") are not covered yet: an `events` table, with `at` and `until` on `note`, is the next extension.
- Of the open problems of [0009](0009-hardware-knowledge-deferred.md): what the engineer said is recorded by an explicit tool call, and corrected by recording it again; facts about the machine itself (sensors, parts) are still not modeled.
