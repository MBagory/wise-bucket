# Install

## Requirements

| | Linux (x86_64, arm64) | macOS (Apple Silicon, Intel) | Windows |
| --- | --- | --- | --- |
| Wise Bucket | ✅ | ✅ | via **WSL2** (see below) |
| Managed PostgreSQL + pgvector | ✅ | ✅ | via WSL2 |
| C toolchain (one-time pgvector build) | `build-essential` (Debian/Ubuntu) or `gcc make` | Xcode Command Line Tools | inside WSL2 |
| Disk | about 250 MB (PostgreSQL, pgvector, empty database) | same | same |

You also need an MCP-capable harness, for example [Claude Code](https://claude.com/claude-code).

## 1. Get the binary

Pre-built releases will be published on GitHub. Until then, build from source with a Rust toolchain ([rustup](https://rustup.rs)):

```sh
git clone https://github.com/MBagory/wise-bucket
cd wise-bucket
cargo install --path crates/wb-server --locked
wise-bucket-server --version
```

`cargo install` puts `wise-bucket-server` in `~/.cargo/bin`.

## 2. Install the C toolchain (once)

Managed mode builds the pgvector extension from its pinned, checksum-verified sources, which takes about 15 seconds.

- **macOS:** `xcode-select --install`
- **Debian / Ubuntu:** `sudo apt install build-essential`
- **Fedora:** `sudo dnf install gcc make`

Using your own PostgreSQL? You do not need a toolchain; see [Bring your own PostgreSQL](configure.md#bring-your-own-postgresql).

## 3. Run setup

```sh
wise-bucket-server setup
```

Continue with [Configure](configure.md), which explains every question `setup` asks.

## Windows: use WSL2

Native Windows support is planned for a later milestone. Today, run Wise Bucket **inside WSL2**, where it behaves exactly as on Linux:

1. Install WSL2 with Ubuntu: `wsl --install -d Ubuntu`.
2. Inside Ubuntu: install Rust and `build-essential`, then follow steps 1–3 above.
3. Run your harness **from WSL** (for example, start `claude` in the WSL terminal, or use VS Code's *WSL* remote), so it can launch the Linux binary.
4. Recordings stored on the Windows side are reachable as `/mnt/c/...` and can be declared as data roots. Access through `/mnt/c` is slower; for large recordings prefer a folder inside the WSL file system (for example `~/robot-logs`).

## Uninstall

```sh
wise-bucket-server db stop           # if it is running
cargo uninstall wb-server
```

Your data stays in the state directory (`wise-bucket-server config path` shows it). Delete it yourself if you no longer need it; Wise Bucket never deletes it.
