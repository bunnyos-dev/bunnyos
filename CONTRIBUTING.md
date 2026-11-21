# Contributing to Bunny Linux

Thanks for looking. This page is the map: where things are, how a change
flows through the code, and the rules everyone follows. What Bunny Linux is
and why is in [docs/DESIGN.md](docs/DESIGN.md); what's next is in
[docs/ROADMAP.md](docs/ROADMAP.md).

## Where to start

1. Read the [Guide](https://bunnyos-dev.github.io/bunnyos/)'s "Start here"
   pages. They're short and say what bunny does for a user, which is the
   spec for the code.
2. Build it and run the read-only commands against the example config:

   ```sh
   cargo build
   ./target/debug/bunny check examples/bunny.toml
   ./target/debug/bunny plan examples/bunny.toml
   ```

   Nothing here changes your system. Commands that would change a system
   take `--root <dir>`; pointed at anything but `/`, they write only inside
   that folder and print the commands instead of running them.
3. Read `crates/bunny/src/apply/mod.rs`: `switch` is a list of named steps,
   and following them is the quickest way to see how everything fits.

## The repository

| Path | What it is |
|---|---|
| `crates/bunny/` | The `bunny` command (Rust). Everything below is in `src/`. |
| `crates/bunny/tests/fixtures/` | Real emerge output, used as test cases for the explanations. |
| `docs/guide/` | The user guide (mdBook), published to GitHub Pages. |
| `docs/DESIGN.md` | Decisions. The source of truth; update it when one changes. |
| `docs/ROADMAP.md` | The to-do list, in the agreed order. |
| `docs/RELEASING.md` | How a release is made. |
| `tools/bunny-vm.sh` | The test VM (UEFI, Secure Boot, TPM, snapshots). |
| `tools/vm-install-gentoo.sh` | Installs Gentoo in the VM with the Bunny Linux disk layout. |
| `tools/convert-layout.sh` | Moves an existing Gentoo on btrfs to the Bunny Linux layout. |
| `tools/canary.sh` | The weekly check against the latest Gentoo tree (run by CI). |
| `tools/build-iso.sh`, `tools/iso/` | Builds the live USB image (the welcome, boot menu, and what runs inside it). |
| `examples/bunny.toml` | A config to try things against. |

The ebuilds live in a separate repository,
[bunny-overlay](https://github.com/bunnyos-dev/bunny-overlay) (GPL-2).

## The code, module by module

`main.rs` holds the command list (clap) and sends each command to its
handler in `commands/`. Everything else is a library module with its own
tests.

**Commands** (`commands/`): what each command prints and asks.

| Module | Commands |
|---|---|
| `switch.rs` | `switch`, `update`, and the summary printed after them |
| `packages.rs` | `search`, `add`, `remove` |
| `undo.rs` | `generations`, `rollback`, `clean` |
| `inspect.rs` | `diff`, `lock`, `explain` |
| `report.rs` | `report` (developer mode) |
| `mod.rs` | Shared: questions (`confirm`), running as root (`elevate`), printing config problems |

**Reading** (never change anything):

| Module | Does |
|---|---|
| `config.rs` | Parses `bunny.toml` and finds mistakes (`bunny check`). |
| `plan.rs` | Compares the config with a system: what a switch would do. |
| `diff.rs` | Config and system side by side, with both ways to fix each difference. |
| `import.rs` | Writes a `bunny.toml` from an existing system. |
| `lock.rs` | `bunny.lock`: exact versions, the pins, and comparing them with the system. |
| `repo.rs` | What's available to install (the repositories' metadata). |
| `status.rs` | `bunny status`: gathering and rendering are kept apart, so rendering is testable. |
| `glsa.rs` | Gentoo security advisories, through `glsa-check`. |
| `notes.rs` | The small notes next to each snapshot, and the last action. |

**Changing a system:**

| Module | Does |
|---|---|
| `apply/` | `bunny switch` itself, step by step (see below). |
| `portage.rs` | Turns the config into Portage's files (world, package.use). |
| `init.rs` | Services, behind the `InitSystem` trait (OpenRC today). |
| `security.rs` | `[security]`: the sudo or doas file and the wheel group. |
| `rollback.rs` | Generations: listing, rolling back, cleaning. |
| `applied.rs` | The copy of the config kept inside the system, so undo puts it back. |
| `boot.rs` | The boot menu's undo entry and the dracut module (`boot/`). |
| `kernels.rs` | A kernel per generation when `/boot` is its own partition. |
| `edit.rs` | Editing `bunny.toml` for `add`/`remove`, keeping comments. |

**Talking to Portage and the user:**

| Module | Does |
|---|---|
| `helper.rs` + `helper/bunny-portage.py` | Asks Portage's own library what emerge would do, as JSON. |
| `emerge.rs` | Reads emerge's output: the install list, progress, messages, build times. |
| `explain.rs` | Turns emerge's messages into plain English with a fix. One recogniser per kind of message. |
| `progress.rs` | The progress line during installs. |
| `interrupt.rs` | Ctrl+C: safe at questions and during installs. |
| `dev.rs` | Developer mode: recording commands and sending reports. |

## How `bunny switch` flows through the files

1. `main.rs` parses the command line and calls `commands::switch`.
2. `commands/switch.rs` first runs itself again through sudo or doas if it
   isn't root (`commands::elevate`). Then it loads the config (`config.rs`),
   checks it, makes the plan (`plan.rs`), shows it and asks.
3. `lock::switch` writes the version pins (`lock.rs`) and calls
   `apply::switch` (`apply/mod.rs`), which runs the steps in order. Each
   starts only once the one before worked:
   - **0.** Add the undo step to the startup image, if the boot menu entry is
     on (`apply/boot_menu.rs`).
   - **1.** `take_snapshot`: a read-only btrfs snapshot. If it fails,
     nothing has changed. With a separate `/boot`, the kernel is copied too
     (`kernels.rs`).
   - **2–3.** `write_package_files`: back up and take over package.use
     (`apply/package_use.rs`), write the world file (`portage.rs`).
   - **4.** `install_and_update` (`apply/install.rs`): ask Portage what it
     would install (`helper.rs`, falling back to `emerge --pretend`), show
     the list, ask again only if it's more than the plan showed, then
     install with a progress line. A refusal or "no" puts the files back.
   - **5.** `apply_security` (`apply/admin.rs`).
   - **6.** `remove_unused`: depclean. Always asks.
   - **7.** `enable_services` (`init.rs`).
   - **8.** `update_boot_menu`: the undo entry for this snapshot.
4. Back in `lock::switch`, the lock is rewritten from what's installed now.
   Then `commands/switch.rs` keeps a copy of the config (`applied.rs`) and
   prints the summary, package messages and tips.

Every external command goes through the `Runner` trait (`apply/runner.rs`):
`SystemRunner` runs it, `PrintRunner` prints it (for `--root`), and the tests
use small fakes that record commands or pretend to be btrfs, dracut or emerge.

## Running the tests

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
mdbook build docs/guide
```

All four run in CI and must pass. Tests never touch the real system: they
use temporary folders as fake roots (`tempfile`) and fake runners. New code
that writes to a system needs a test against a fake root.

## Testing in the VM

Anything that changes a real system is also tried in a VM before it's
merged. `tools/bunny-vm.sh` runs one with UEFI, Secure Boot (in setup mode),
a TPM and 3D graphics, and keeps snapshots so each test starts clean. The
guide's [Testing in a virtual machine](docs/guide/src/reference/testing-in-a-vm.md)
page has the setup. A typical test:

```sh
cargo build --release
tools/bunny-vm.sh restore clean-install
tools/bunny-vm.sh run
tools/bunny-vm.sh put target/release/bunny     # in a second terminal
tools/bunny-vm.sh ssh
```

`VM=laptop` gives a second VM shaped like an encrypted laptop (LUKS2, `/boot`
on the EFI partition).

## Rules

These apply to everyone, people and AI sessions alike (`CLAUDE.md` has the
full list for AI sessions).

- **Version 1 scope and OpenRC only**, unless the maintainer asks otherwise.
  Init-system code stays behind the `InitSystem` trait.
- **Safety:** bunny runs as root. Anything that writes to a system accepts
  `--root`, shows a preview before destructive changes, and asks. Nothing
  touches disks outside of install.
- **Plain English:** messages say what happened and what to do. Never print
  raw tool output as the only explanation.
- **No personal information** in the repo: no real user names, host names,
  emails or paths, even as examples. Use `you`, `desktop`, `laptop`.
- **The Guide moves with the code:** a change to what a command does or
  prints updates the matching guide page in the same pull request. Describe
  what bunny does today; mark planned things as planned.
- **Small modules with unit tests** over large untested ones.
- **CHANGELOG:** changes people would notice get a line under
  `## [Unreleased]`, in plain words.
- **One pull request per roadmap item**, small enough to test in about 15
  minutes. Every description ends with **"How to test in the VM"**: exact
  commands, one per line, and what the output should look like.
- **Licence:** GPL-3.0 for this repository (the overlay's ebuilds are GPL-2).
