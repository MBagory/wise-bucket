# 0018. Extraction starts on file events

**Date:** 2026-10-11 · **Status:** Accepted

## Context

An agent session walked the data roots every minute and extracted a file once it had not changed for 5 seconds. A downloaded file waited up to a minute before it could be queried: the PX4 ULog demo was extracted 50 seconds after its download finished.

## Decision

- Each session watches its data roots with the operating system's file events ([notify](https://github.com/notify-rs/notify): FSEvents on macOS, inotify on Linux).
- A file or rosbag2 folder **renamed into place** is extracted at once: the rename is the last step of a download (demo samples, browsers, rsync) or a move.
- A file **written in place** (`cp`, a logger) is extracted once it has not changed for 5 seconds. A change sets one timer for that delay; events in the meantime are covered by the same walk.
- The roots are also walked at startup and on each `server_info` call.
- The walk every minute remains only when a root cannot be watched.

## Alternatives

- **Walking every minute:** simple, but a new file waits up to a minute.
- **Extracting at every event:** a file still being written is extracted again at each batch of writes.
- **inotify's close-after-write event:** marks the end of an in-place write on Linux, but FSEvents has no equivalent. Add it if in-place writers on Linux need to skip the 5 seconds.
- **Notifying sessions from Wise Bucket's own commands** (`demo`, `roots add`): misses files copied in by anything else.

## Consequences

- A finished download is queryable as soon as its extraction ends; a file written in place, 5 seconds after its last write.
- File events do not report changes made by another computer on a network share; those recordings are found at the next `server_info` call.
- A root declared during a session is watched from the next `server_info` call.
- Each walk still covers every root; a root receiving many files in a row is walked once per file renamed into place.
