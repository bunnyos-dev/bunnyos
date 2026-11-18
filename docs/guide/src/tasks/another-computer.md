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
sudo git init
sudo git add bunny.toml bunny.lock hosts
sudo git commit -m "My system"
```

**Push every commit somewhere else** (a private GitHub repository, your
NAS): `/etc/bunny` is part of the system, so undoing a change takes the
repository back too, and commits made after that point would only survive
in the old system that `bunny clean` later deletes. With a remote, they're
always there:

```sh
sudo git remote add origin <your repository's address>
sudo git push -u origin HEAD
```

After a rollback, `sudo git fetch` and `git log origin/HEAD` show everything
you'd committed.

## Things to know

- **Old versions disappear from Gentoo.** Gentoo removes old versions from its
  package tree after a while. If the lock asks for a version that's gone,
  emerge can't install it. Run `bunny update` on the new machine to move to
  current versions.
- **Different hardware.** Packages that only make sense on one machine (a
  graphics driver, Bluetooth tools) go in that machine's host file,
  `hosts/<computer name>.toml`, instead of the shared list. See
  [host files](../reference/config.md#host-files).
- **Installing from scratch.** On a new computer, `bunny install --config
  github:you/my-system` installs Bunny Linux straight from your repository
  (see [bunny install, in detail](../reference/install.md)).
  Per-machine settings are planned.
