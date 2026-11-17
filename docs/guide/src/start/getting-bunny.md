# Getting bunny

> Today, bunny is installed by building it from source on a Gentoo system. A
> ready-made BunnyOS installer is planned.

## What you need

- **Gentoo with OpenRC.**
- **Root (`/`) on btrfs**, with a `/.snapshots` folder that's its own btrfs
  subvolume. That's what lets bunny undo changes. Without it bunny still
  works, but switches can't be rolled back (it warns you each time). The
  recommended layout is in [The disk layout](../reference/disk-layout.md).
- **doas** set up for your user. bunny asks for your password through it when
  it needs to change the system.
- **Rust**, to build bunny. The prebuilt one is quickest:

  ```sh
  doas emerge -av dev-lang/rust-bin
  ```

> **Try it in a virtual machine first.** The repository has a script that
> sets one up with the right layout. See
> [Testing BunnyOS in a virtual machine](../reference/testing-in-a-vm.md).

## Build it

```sh
git clone https://github.com/bunnyos-dev/bunnyos
cd bunnyos
cargo build --release
doas install -m 755 target/release/bunny /usr/local/bin/bunny
```

Check it works:

```sh
bunny --help
```

## Update bunny itself

```sh
cd bunnyos
git pull
cargo build --release
doas install -m 755 target/release/bunny /usr/local/bin/bunny
```

Next: [Your first five minutes](first-five-minutes.md).
