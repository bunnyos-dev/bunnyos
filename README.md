# BunnyOS

A Gentoo-based operating system with NixOS-style reproducibility and a
polished, quiet experience from power button to desktop.

- **Gentoo underneath:** OpenRC, USE flags, compile what you want.
- **One file describes your machine:** `bunny.toml` + a lockfile. Copy them to
  another computer and get the same system.
- **Rollback:** every change is a generation you can boot back into.
- **Errors in plain English:** with suggested fixes instead of raw emerge output.

> **Status:** early development. Nothing here is ready for real machines yet.
> Test only in a VM (see `tools/bunny-vm.sh`).

## Progress

| Phase | Status |
|---|---|
| 1. Read a system (`import`, `check`, `plan`) | ✅ Done |
| 2. Change a system (`switch`) | ✅ Done |
| 3. Undo (`generations`, `rollback`, `clean`) | ✅ Done (boot-menu undo to do) |
| 4. Same system everywhere (lockfile) | ✅ Done |
| 5. Understandable errors | 🔶 Part 1 done |
| 6. Fast installs (package cache) | ⬜ |
| 7. The look (boot animation, GRUB, login) | ⬜ |
| 8. Security + gaming | ⬜ |
| 9. Installer + ISO | ⬜ |

Version 1 (phases 1–5) is roughly two-thirds done. Details and the full to-do list
are in the [roadmap](docs/ROADMAP.md).

## Documentation

- **[The BunnyOS Guide](https://bunnyos-dev.github.io/bunnyos/)**: how to use
  bunny, from first steps to every setting. Source in `docs/guide/`.

- [Design](docs/DESIGN.md): what BunnyOS is, every decision so far, and the
  version 1 scope.
- [Roadmap](docs/ROADMAP.md): what's done and what's next.

## Building `bunny`

```sh
cargo build --release
./target/release/bunny --help
```

Try it against the example config (read-only, changes nothing):

```sh
./target/release/bunny check examples/bunny.toml
./target/release/bunny plan examples/bunny.toml
```

## Testing in a VM

```sh
tools/bunny-vm.sh run path/to/gentoo-install.iso
```

The VM has UEFI, Secure Boot (in Setup Mode), a TPM 2.0 chip and 3D graphics.
See the comments at the top of the script for snapshots and other commands.

## How it's made

BunnyOS is designed by its maintainer and largely written with AI assistance.
Everything is tested in a VM before it's considered working.

## Licence

GPL-3.0. See [LICENSE](LICENSE).
