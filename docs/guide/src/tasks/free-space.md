# Free up space

Every generation keeps the files that changed since, so over time they use
disk space. `bunny clean` removes old ones:

```sh
bunny clean
```

```
Old generations to remove (keeping the newest 5):
  - Nov 1, 10:00 AM   bunny-20261101-100000
Old systems left behind by rollbacks:
  - @replaced-20261116-212536   (with /srv inside it)

This can't be undone: you won't be able to roll back to these anymore.
Remove them? [y/N]
```

Type **`y`** to remove them. Here **Enter means no**, because this one can't
be undone.

## Keep more or fewer

```sh
bunny clean --keep 10
bunny clean --keep 2
```

If `/boot` is its own partition, `clean` also removes the kernel copies that
only the removed generations used.

bunny always keeps:

- at least the newest one
- the generation you're currently running a copy of (after a rollback)
- the system you're running right now (even before rebooting after a rollback)
