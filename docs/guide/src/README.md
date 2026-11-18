# Welcome

This is the guide to **Bunny Linux** and its tool, **bunny**.

It has two parts:

- **Start here** and **Everyday tasks** assume you know how to use Linux but
  have never used Gentoo's or NixOS's way of doing things. Every page is one
  task, with commands you can copy.
- **Going further** is for when you want to know exactly what bunny does,
  every setting, and how it works underneath.

Words that might be new are explained in the [Glossary](glossary.md).

> **sudo or doas?** Commands in this guide use `sudo`, the Bunny Linux default. If
> your system uses `doas`, type `doas` instead. bunny's own messages always
> use whichever one you have.

> **Bunny Linux is in early development.** It installs from a live USB to a
> minimal system with a terminal ([Install Bunny Linux](start/installing.md)),
> and the `bunny` tool also works on an existing Gentoo install. The
> graphical installer, Bunny Desktop and the boot experience come later. Try it in a [virtual machine](reference/testing-in-a-vm.md) before
> using it on a computer you care about.

## The short version

```sh
bunny status                  # is everything in order?
bunny search browser          # find something
bunny add firefox             # install it
bunny remove firefox          # remove it
bunny update                  # update everything
bunny rollback list           # see earlier versions of your system
bunny rollback 3              # go back to one
```

Every command that changes your system shows you what it's going to do and
asks first. Every change can be undone.
