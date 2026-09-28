# Security policy

## Supported versions

Wise Bucket is alpha software. Security fixes are made on the latest release and the `main` branch.

## Reporting a vulnerability

**Please do not open a public issue.** Report privately through GitHub: *Security › Report a vulnerability* on the repository (GitHub Security Advisories). We aim to acknowledge reports within 5 working days and to agree on a disclosure timeline with you.

Useful details: affected version (`wise-bucket-server --version`), platform, steps to reproduce, and impact.

## Threat model (summary)

Wise Bucket runs locally, for one engineer, next to an AI harness.

**What Wise Bucket protects:**

- **Data roots:** only folders declared by the engineer can be read; symlinks escaping a root are refused; no MCP tool can add a root.
- **Managed database:** listens only on a Unix socket in a directory with mode 0700; generated passwords are stored in a 0600 file; no TCP listener.
- **Supply chain at setup:** PostgreSQL binaries and pgvector sources are pinned by version and verified with SHA-256 before use.
- **Secrets:** an external database URL is never written to configuration files; `config show` masks passwords.

**Out of scope / residual risks:**

- The AI harness's own tools (shell, file access) are not controlled by Wise Bucket. Restrict them in the harness if you need containment.
- A local attacker running as your user can read your files, including Wise Bucket's state directory.
- Tool results sent by your harness to its model follow the harness's and model provider's terms.
