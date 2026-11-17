# Welcome

This is the guide to **BunnyOS** and its tool, **bunny**.

It has two parts:

- **Start here** and **Everyday tasks** assume you know how to use Linux but
  have never used Gentoo's or NixOS's way of doing things. Every page is one
  task, with commands you can copy.
- **Going further** is for when you want to know exactly what bunny does,
  every setting, and how it works underneath.

Words that might be new are explained in the [Glossary](glossary.md).

> **BunnyOS is in early development.** The `bunny` tool works on an existing
> Gentoo install today. The installer, the boot experience and the rest come
> later. Try it in a [virtual machine](reference/testing-in-a-vm.md) before
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
