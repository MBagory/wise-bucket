# Security policy

## Supported versions

Wise Bucket is alpha software. Security fixes are made on the latest release and the `main` branch.

## Reporting a vulnerability

**Please do not open a public issue.** Report privately through GitHub: *Security › Report a vulnerability* on the repository (GitHub Security Advisories). We aim to acknowledge reports within 5 working days and to agree on a disclosure timeline with you.

Useful details: affected version (`wisebucket --version`), platform, steps to reproduce, and impact.

## Privacy

- **Stays on your machine:**
  - Your recordings: Wise Bucket reads them in place and never uploads, modifies or moves them.
  - The store: a decoded, columnar copy of your recordings' signals, log messages, parameters and header information (images and point clouds excepted), in the state directory (`<state directory>/store`). It is derived data: delete it any time. Recordings you delete from your folders keep their data in the store until you delete it.
- **Reaches your model provider:** only the tool results your agent sends to its own model, under your agent's terms. Wise Bucket's results are bounded: about 60 KB of rows per query, text cells cut at 1,000 characters. They contain whatever the agent's queries select from your recordings, plus folder names and paths.
- **No model calls:** Wise Bucket never contacts an LLM.
- **No network use at runtime:** only `demo` downloads files (public samples), each verified against a pinned SHA-256 checksum.

## Threat model (summary)

Wise Bucket runs locally, for one engineer, next to an AI harness.

**What Wise Bucket protects:**

- **Data roots:** only folders declared by the engineer can be read; symlinks escaping a root are refused; no MCP tool can add a root.
- **The agent's SQL:** read-only by construction. The query engine sees only the store's tables; DDL, writes, `COPY` and other statements are refused before planning, file-reading functions and external tables are unavailable, and each query is capped in time (30 s), memory (1 GB, no spilling to disk) and result size (about 60 KB).
- **Untrusted recordings:** each recording is decoded in a separate, low-priority process, so a malformed file that crashes a decoder cannot take down the agent's session; an extraction running longer than an hour is stopped. rosbag2 SQLite files are opened read-only and immutable, so not even a journal is written next to them.
- **Supply chain:** sample downloads are pinned by commit and verified with SHA-256; bundled ROS 2 message definitions are pinned to specific commits.

**Out of scope / residual risks:**

- The AI harness's own tools (shell, file access) are not controlled by Wise Bucket. Restrict them in the harness if you need containment.
- A local attacker running as your user can read your files, including Wise Bucket's state directory.
- A malformed recording can make its extraction process use a lot of memory: extraction processes have no memory limit yet.
- Tool results sent by your harness to its model follow the harness's and model provider's terms.
