# When something goes wrong

## bunny explains it for you

When emerge (Portage's tool) stops during a `switch` or `update`, bunny reads
its message and tells you what it means:

```
emerge stopped, so the changes weren't installed.

Here's what happened:

✗ swtpm needs gnutls built with these features turned on: pkcs11, tools.
  This only rebuilds gnutls; nothing else changes.
  Fix: add this to your bunny.toml, then run `bunny switch` again:
       [pkg."net-libs/gnutls"]
       use = ["pkcs11", "tools"]

Nothing is lost: your old files are backed up in /var/lib/bunny/backups/…
The system before this change is saved in /.snapshots/… (see `bunny rollback list`).
```

Do what the **Fix** says and run the same command again.

## If you press Ctrl+C

It's safe to stop bunny at any point:

- **At a question** ("Apply these changes?", "Install these?"), Ctrl+C
  counts as **no**: nothing is installed, and `bunny.toml` is put back as it
  was.
- **While it's installing**, emerge stops, and bunny says how far it got:

  ```
  ✗ Stopped with Ctrl+C while installing.
    101 of 102 packages finished installing.
    www-client/firefox was in progress and didn't.
    Your config still lists www-client/firefox, so `bunny status`
    will show it as not applied yet.

  Two ways on from here:
    • Undo the whole attempt: `bunny rollback`, then restart. …
    • Finish it later: `bunny switch` picks up where this stopped.
  ```

## Messages from outside bunny

If you ran `emerge` yourself and got something confusing, paste it into
bunny:

```sh
bunny explain
```

Paste the message, then press **Ctrl+D**. Or save it to a file and run
`bunny explain file.txt`.

## What bunny recognises

| emerge says | What it means |
|---|---|
| "USE changes are necessary" / "Change USE" | A program needs another one built with certain features. bunny gives you the exact lines to add. |
| keyword changes with `~amd64` | It's only available as a **testing** version. |
| keyword changes with `9999` or `**` | ⚠ emerge wants **unreleased development code**. Don't accept it; bunny tells you how to find the real problem. |
| "have been masked" | Gentoo **blocked it on purpose**; bunny shows Gentoo's reason. |
| "slot conflict" / "skipped due to a dependency conflict" | Two programs need **different versions** of the same thing. Each conflict is shown on its own, with which version each program wants. When it's a prebuilt package wanting an older version, that's Gentoo's prebuilt packages being built at different times: `bunny update` usually fixes it within days, or compile that one package (`prebuilt = false`). |
| `[blocks B]` | Two programs **can't be installed together**. |
| "Fetch failed" | A **download failed**: check your connection, try later. |
| `Traceback (most recent call last)` | **emerge itself crashed** (an error inside Portage, not your config), usually while downloading. Run the same command again; bunny shows Portage's one-line error, not the whole Python trace. |
| "no ebuilds to satisfy" / "don't exist" | **No package by that name**, usually a typo. |

If bunny doesn't recognise a message, it says so instead of guessing. Please
[share it](https://github.com/bunnyos-dev/bunnyos/issues) so it can learn it.

## Typos are caught early

If `bunny.toml` asks for a package that doesn't exist, `switch` stops before
changing anything:

```
✗ These packages don't exist in your package repositories:
  ? app-misc/tmuxx
```

## If emerge suggests something risky

emerge sometimes offers to "fix" a problem by writing changes to
`/etc/portage` and asks "Would you like to add these changes to your config
files?". If the suggestion includes `9999` or `**`, answer **No**. Those are
untested development versions. Run the command again and look at the actual
problem (or paste it into `bunny explain`).
