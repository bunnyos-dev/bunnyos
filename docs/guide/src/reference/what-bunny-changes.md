# What bunny changes on your system

Everything bunny writes, so nothing is a mystery.

| File | What it is |
|---|---|
| `/var/lib/portage/world` | Gentoo's list of wanted packages. bunny writes it from `packages`, plus the `[security]` tool and bunny's own package. |
| `/etc/portage/package.use/zz-bunny` | USE flags from your config. Named so Portage reads it **last**, so it wins. |
| `/etc/portage/package.use/00-bunny-kept` | package.use lines bunny can't manage yet (version ranges like `<pkg-2`, wildcards), copied word for word. |
| `/etc/portage/package.accept_keywords/zz-bunny` | Testing versions allowed by `[pkg] keywords`. Only when your config has some. Other files in the folder are left alone; a single-file `package.accept_keywords` is moved into the folder as `00-yours`. |
| `/etc/portage/repos.conf/bunny-<name>.conf` | One per repository in [`repositories`](config.md#repositories), with its git address from Gentoo's list. Removed when the repository leaves your config. A single-file `repos.conf` is moved into the folder as `00-yours.conf`. |
| `/var/db/repos/<name>` | The repositories' files, downloaded with `emaint sync`. |
| `/etc/portage/package.mask/zz-bunny-lock` | The [version pins](lock-and-pins.md) from `bunny.lock`. |
| `/etc/sudoers.d/bunny` or `/etc/doas.conf` | Admin rights for the `wheel` group, from [`[security]`](config.md#security). Only if that section is in your config. |
| `/etc/group` (`wheel` only) | `admins` from `[security]` are added with `gpasswd`. Nobody is ever removed. |
| `/etc/runlevels/default/*` | Services from `services` (and `[desktop]`), added with `rc-update`. `elogind` goes in `/etc/runlevels/boot/`. |
| `bunny.lock` (next to the config) | Exact versions. |
| `/.snapshots/bunny-*` | Generations, plus small `.toml` notes. |
| `/usr/lib/dracut/modules.d/90bunny-rollback/` | The boot menu's undo step, with [`[boot] undo_entry`](config.md#boot). Added to the startup image (`/boot/initramfs-<version>.img`), which is rebuilt once; the old image goes to the backup folder. |
| `/boot/bunny/` | Only when `/boot` is its own partition: a copy of each generation's kernel files, so undo can put the kernel back. See [Generations](generations.md#when-boot-is-its-own-partition). `bunny clean` removes copies nothing uses. |
| `/etc/init.d/bunny-finish-undo` | Added with the undo step, in the `boot` runlevel. After an undo from the boot menu on a system where `/boot` is its own partition, it puts `/boot`'s kernel back early in the first start. Does nothing otherwise. |
| `/var/lib/bunny/boot-undo`, `boot-undo-done.toml` | The note an undo from the boot menu leaves for that service, and what the service did (for `bunny status`). |
| `/boot/grub/custom.cfg` | The boot menu's undo entry, between `### BEGIN bunny` and `### END bunny`. The rest of the file is kept. |
| `/var/lib/bunny/news-seen` | How many unread Gentoo news items bunny last told you about, so the tip only comes back when there's something new. |
| `/var/lib/bunny/applied/` | A copy of the config and lock from the last successful switch, so undoing a later change can put them back. Part of the system, so each generation carries its own. |
| `/var/log/bunny/<time>-emerge.log` | emerge's full output from each switch or update. |
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
| See what would be installed | bunny's Portage helper, or `emerge --pretend --update --deep --newuse --verbose @world` |
| Install / update | `emerge --update --deep --newuse --quiet-build=y @world` |
| See what's unused | `emerge --depclean --pretend` |
| Remove unused | `emerge --depclean` (after bunny asks, always) |
| New repositories | `wget -qO- https://api.gentoo.org/overlays/repositories.xml` (Gentoo's list), then `emaint sync --repo <name>`, right after the snapshot |
| Services | `rc-update add <name> default` (`boot` for elogind), `rc-service <name> start` |
| Admin rights | `visudo -cf <file>` or `doas -C <file>` to check, `gpasswd -a <user> wheel` |
| Update | `emerge --sync` first |

bunny asks emerge what it would do first (`--pretend`, which changes nothing),
shows that as a short list, and asks. If emerge refuses at that point,
bunny puts the world file and package.use back, so nothing has changed. While
installing it shows one progress line; emerge's full output is saved to
`/var/log/bunny/<time>-emerge.log`. With `--verbose`, emerge runs with
`--verbose --ask` and its output is shown live instead, as it used to be.

Either way, bunny reads emerge's output to explain any problem, turns its
notices (news, reminders) into short tips, and shows the messages packages
leave after installing.

Unused packages are looked for after removing something from your config,
after USE flag changes, and after updates, since all three can leave
dependencies that nothing needs anymore.

## What bunny leaves alone

`/etc/portage/make.conf`, other files in `package.accept_keywords` and
`repos.conf`, the kernel config, and everything in `/etc` outside the files
above.
