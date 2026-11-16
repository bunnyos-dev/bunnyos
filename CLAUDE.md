# Working on BunnyOS

Read `docs/DESIGN.md` first. It is the source of truth; update it when a
decision changes.

## Rules

- **Version 1 scope only** unless the maintainer asks otherwise (see
  "Version 1 scope" in the design doc).
- **OpenRC only.** Keep init-system code behind the `InitSystem` trait.
- **Safety:** bunny runs as root. Anything that writes to a system must accept
  `--root` so it can be tested against a fake root, and must show a preview
  before destructive changes. Never touch disks outside of install.
- **Plain English:** user-facing messages explain what happened and what to do.
  Never print raw tool output as the only explanation.
- **Commands for the maintainer use `doas`, not `sudo`.**
- **No personal information** in the repo: no real usernames, hostnames, emails
  or paths from the maintainer's machines, even as examples. Use `you`,
  `desktop`, `laptop`.
- **Licence:** GPL-3.0 for this repo. Ebuilds in the overlay are GPL-2.

## Code

- Rust workspace; the CLI lives in `crates/bunny`.
- `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` must pass.
- Prefer small modules with unit tests over large untested ones.
