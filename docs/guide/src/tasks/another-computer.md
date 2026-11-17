# Set up the same system on another computer

Your system is two files:

- `/etc/bunny/bunny.toml`: what you want
- `/etc/bunny/bunny.lock`: the exact versions you have

## Copy them over

On the new computer (with Gentoo, OpenRC and bunny set up, see
[Getting bunny](../start/getting-bunny.md)), put both files in `/etc/bunny/`,
then:

```sh
bunny switch
```

bunny installs everything in `bunny.toml`, held at the versions in
`bunny.lock`.

## Keeping it in git

A good habit is to keep both files in a (private) git repository, so every
machine pulls the same setup and you have the history of every change:

```sh
cd /etc/bunny
doas git init
doas git add bunny.toml bunny.lock
doas git commit -m "My system"
```

## Things to know

- **Old versions disappear from Gentoo.** Gentoo removes old versions from its
  package tree after a while. If the lock asks for a version that's gone,
  emerge can't install it. Run `bunny update` on the new machine to move to
  current versions.
- **Different hardware.** Packages that only make sense on one machine (a
  graphics driver, Bluetooth tools) are best kept out of the shared list.
  Per-machine settings are planned.
