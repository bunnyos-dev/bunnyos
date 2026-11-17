# Your first five minutes

You already have a working Gentoo system. These steps turn it into one that
bunny manages, **without changing anything** until the last step.

## 1. Write down your current system

```sh
doas mkdir -p /etc/bunny
doas bunny import -o /etc/bunny/bunny.toml
```

bunny looks at what's installed, which services start at boot and your USE
flags, and writes them into `/etc/bunny/bunny.toml`. `/etc/bunny/bunny.toml` is
where every bunny command looks by default, so you won't need to type the path.

It tells you if it found anything it couldn't bring over (they're listed at
the end of the file as comments, and keep working where they are).

## 2. Have a look

```sh
less /etc/bunny/bunny.toml
```

It's your whole system: a list of packages, a list of services, and USE
flags per package. Delete things you don't recognise later, not now.

## 3. Check it

```sh
bunny check
```

This catches mistakes in the file and tells you how to fix each one.

## 4. See what bunny would do

```sh
bunny plan
```

Right after `import` this should say:

```
✓ This system already matches your config. Nothing to do.
```

`plan` never changes anything. Use it whenever you're unsure.

## 5. Let bunny take over

```sh
bunny switch
```

Since your config matches the system, there's nothing to install. bunny
creates `bunny.lock` (the exact versions you have now) and from then on keeps
your packages at those versions until you choose to update.

> The first real switch also **takes over your USE flag files** in
> `/etc/portage/package.use/`: they're moved to a backup and replaced by one
> file generated from `bunny.toml`. Nothing is lost, since the flags were
> imported in step 1. Lines bunny can't manage yet are kept word for word.
> Details: [What bunny changes on your system](../reference/what-bunny-changes.md).

## Done

From now on:

- [Install a program](../tasks/install.md) with `bunny add`
- [Update](../tasks/update.md) with `bunny update`
- [Undo](../tasks/undo.md) with `bunny rollback`
