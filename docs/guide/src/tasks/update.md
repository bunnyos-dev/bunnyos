# Update your system

```sh
bunny update
```

This is the **only** command that moves your packages to newer versions.
Everything else (`switch`, `add`, `remove`) keeps installed packages at the
versions in `bunny.lock`.

`bunny update`:

1. syncs the Gentoo package tree (downloads the latest list of packages)
2. saves a snapshot of your system
3. updates everything
4. writes the new versions to `bunny.lock`, and shows what changed:

```
✓ Updated.
  The system before this update is saved in /.snapshots/bunny-20261117-091500.
  Updated /etc/bunny/bunny.lock:
    ~ www-client/firefox: 133.0 → 134.0
    ~ gentoo package tree: Mon, 16 Nov 2026 00:45:00 +0000 → Tue, 17 Nov 2026 06:45:00 +0000
```

## How long it takes

Most packages come prebuilt from Gentoo, so an update is usually a few
minutes. Packages you've given custom USE flags have to be compiled, and big
ones (browsers, compilers) can take a while.

## Didn't like the update?

It's one command to go back to the system from before it: see
[Undo a change](undo.md).

## How often?

Once a week is a good rhythm. Security fixes arrive through updates, so
don't leave it for months.
