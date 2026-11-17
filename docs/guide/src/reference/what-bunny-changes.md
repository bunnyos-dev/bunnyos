# What bunny changes on your system

Everything bunny writes, so nothing is a mystery.

| File | What it is |
|---|---|
| `/var/lib/portage/world` | Gentoo's list of wanted packages. bunny writes it from `packages`. |
| `/etc/portage/package.use/zz-bunny` | USE flags from your config. Named so Portage reads it **last**, so it wins. |
| `/etc/portage/package.use/00-bunny-kept` | package.use lines bunny can't manage yet (version ranges like `<pkg-2`, wildcards), copied word for word. |
| `/etc/portage/package.mask/zz-bunny-lock` | The [version pins](lock-and-pins.md) from `bunny.lock`. |
| `/etc/sudoers.d/bunny` or `/etc/doas.conf` | Admin rights for the `wheel` group, from [`[security]`](config.md#security). Only if that section is in your config. |
| `/etc/group` (`wheel` only) | `admins` from `[security]` are added with `gpasswd`. Nobody is ever removed. |
| `/etc/runlevels/default/*` | Services from `services`, added with `rc-update`. |
| `bunny.lock` (next to the config) | Exact versions. |
| `/.snapshots/bunny-*` | Generations, plus small `.toml` notes. |
| `/var/lib/bunny/backups/<time>/` | Copies of the files above, from before each switch. |

## The package.use takeover

The first switch that changes something moves your own files in
`/etc/portage/package.use/` into the backup folder, and writes `zz-bunny`
from `bunny.toml` instead. Since `bunny import` copies your flags into the
config, nothing is lost:

- Lines tied to a version (`>=media-libs/foo-1.2 flag`, what
  `emerge --autounmask-write` writes) are imported for all versions.
- Lines bunny can't express yet go to `00-bunny-kept`, unchanged.

**From then on, change USE flags in `bunny.toml`**, not in
`/etc/portage/package.use`.

## What bunny runs

| Step | Command |
|---|---|
| Snapshot | `btrfs subvolume snapshot -r / /.snapshots/bunny-<time>` |
| Install / update | `emerge --update --deep --newuse --verbose --ask @world` |
| Remove unused | `emerge --depclean --ask` (always asks) |
| Services | `rc-update add <name> default`, `rc-service <name> start` |
| Admin rights | `visudo -cf <file>` or `doas -C <file>` to check, `gpasswd -a <user> wheel` |
| Update | `emerge --sync` first |

emerge's output is shown live, as usual. bunny keeps a copy to explain any
problem, and turns its notices (news, reminders) into short tips at the end.

## What bunny leaves alone

`/etc/portage/make.conf`, `package.accept_keywords`, `repos.conf`, the
kernel config, and everything in `/etc` outside the files above.
