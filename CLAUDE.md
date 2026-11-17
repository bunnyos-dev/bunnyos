# Working on BunnyOS

Read `docs/DESIGN.md` first. It is the source of truth; update it when a
decision changes. `docs/ROADMAP.md` is the to-do list.

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

## The Guide

- `docs/guide/` is the user guide (mdBook). **Any change to what a command
  does or prints updates the matching guide page in the same PR.**
- "Start here" and "Everyday tasks" pages: plain words, one task per page,
  copy-paste commands, written for an experienced Linux user new to Gentoo's
  and NixOS's ideas. "Going further": exact behaviour and every option.
- Describe what bunny does today; mark planned things as planned.
- Check it builds: `mdbook build docs/guide`.

## Code

- Rust workspace; the CLI lives in `crates/bunny`.
- `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test` must pass.
- Prefer small modules with unit tests over large untested ones.

## Daily work sessions

- Work on the first unchecked, unblocked item in `docs/ROADMAP.md`.
- One branch and one pull request per item (`daily/<short-name>`). Never push
  to `main` and never merge; the maintainer tests in the VM and merges.
- If two or more daily PRs are still open, don't start a new item. Leave a
  short comment on the oldest one instead.
- Mark the item `[~]` in the PR's copy of the roadmap.
- Every PR description ends with **"How to test in the VM"**: exact commands,
  one per line, using `tools/bunny-vm.sh restore clean-install`, `put`, `ssh`,
  and saying what the output should look like. Commands that ask questions go
  on their own line.
- Keep PRs small enough to test in about 15 minutes.
- When working live with the maintainer: once they've confirmed a PR's VM
  test passed, merge it for them (squash) and update the roadmap.
