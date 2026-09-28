# FAQ

### Does Wise Bucket call an LLM or use my API key?
No. Wise Bucket never calls a model. Your harness (Claude Code, Cline, …) owns the model, your key and your token budget.

### Do my recordings leave my machine?
No. Wise Bucket reads them in place. Only tool results that your harness sends to its model leave your machine; see [Trust and privacy](philosophy/trust-and-privacy.md).

### Why PostgreSQL? Do I have to install it?
PostgreSQL with pgvector gives Wise Bucket reliable concurrent storage plus semantic and full-text search, which is a core feature from the next milestone. You do not install it yourself: `setup` downloads a private copy, verifies it, and Wise Bucket starts and stops it as needed. You can also use your own server.

### Is PostgreSQL always running in the background?
No. It starts with your first harness session (about 2 seconds) and stops when the last one ends. Set `keep_running = true` under `[database]` in your user config, or `WB_KEEP_RUNNING=true`, to keep it up.

### Why does setup need a C compiler?
pgvector is distributed as source code. `setup` builds the pinned, checksum-verified release once, in about 15 seconds, against the exact PostgreSQL it downloaded. Using your own PostgreSQL avoids this step.

### Can the AI read files outside my data roots through Wise Bucket?
No. Wise Bucket only reads inside declared roots, refuses symlinks that escape them, and has no tool to add a root. Your harness's own tools (for example a shell) are outside Wise Bucket's control; see [Trust and privacy](philosophy/trust-and-privacy.md).

### Can I have several recording folders? On a NAS?
Yes: declare as many roots as you like with `wise-bucket-server roots add <name> <path>`, including network mounts. Roots cannot overlap.

### Can I use several Claude Code windows at the same time?
Yes. Each window starts its own Wise Bucket process; they share the same database.

### Does it work on Windows?
Through WSL2 today; see [Install](get-started/install.md#windows-use-wsl2). Native Windows support is planned.

### Where is my data, and how do I back it up?
`wise-bucket-server config path` shows the locations. `wise-bucket-server backup wb.dump` writes a backup; `restore wb.dump --yes` restores it. Recordings are never part of the backup: they stay where they are.

### What can it do today?
Milestone M0 is the foundation: install, database, data roots, harness connection and `server_info`. The roadmap in the README lists what comes next: knowledge, investigations, ROS 2 logs, then metrics and search.
