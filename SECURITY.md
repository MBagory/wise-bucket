# Security policy

## Supported versions

Wise Bucket is alpha software. Security fixes are made on the latest release and the `main` branch.

## Reporting a vulnerability

**Please do not open a public issue.** Report privately through GitHub: *Security › Report a vulnerability* on the repository (GitHub Security Advisories). We aim to acknowledge reports within 5 working days and to agree on a disclosure timeline with you.

Useful details: affected version (`wisebucket --version`), platform, steps to reproduce, and impact.

## Privacy

- **Stays on your machine:**
  - Your recordings: Wise Bucket reads them in place and never uploads, copies or modifies them.
  - The database: it listens only on a Unix socket in a private directory, never on the network.
  - Credentials: generated database passwords live in `<state directory>/secrets/db.toml` (mode 0600).
- **Reaches your model provider:** only the tool results your agent sends to its own model, under your agent's terms. Wise Bucket's results are bounded summaries: today, versions, the project name, and recording-folder names and paths.
- **No model calls:** Wise Bucket never contacts an LLM.
- **No network use at runtime:** only `setup` downloads files (PostgreSQL), each verified against a pinned SHA-256 checksum.

## Threat model (summary)

Wise Bucket runs locally, for one engineer, next to an AI harness.

**What Wise Bucket protects:**

- **Data roots:** only folders declared by the engineer can be read; symlinks escaping a root are refused; no MCP tool can add a root.
- **Managed database:** listens only on a Unix socket in a directory with mode 0700; generated passwords are stored in a 0600 file; no TCP listener.
- **Supply chain at setup:** PostgreSQL binaries are pinned by version and verified with SHA-256 before use.


**Out of scope / residual risks:**

- The AI harness's own tools (shell, file access) are not controlled by Wise Bucket. Restrict them in the harness if you need containment.
- A local attacker running as your user can read your files, including Wise Bucket's state directory.
- Tool results sent by your harness to its model follow the harness's and model provider's terms.
