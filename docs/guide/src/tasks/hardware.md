# Graphics cards and CPU features

Gentoo builds programs for the hardware you say you have. Two settings
matter most:

- **Graphics cards** (Gentoo calls them `VIDEO_CARDS`): which graphics
  drivers Mesa and friends are built with: `intel`, `nvidia`, `amdgpu`,
  `radeonsi`…
- **CPU features** (`CPU_FLAGS_X86`): which instructions your processor has
  (`aes`, `avx2`…), so codecs and crypto libraries can use them.

Both go in a `[hardware]` section. They differ from one computer to the
next, so put it in **this computer's host file**,
`/etc/bunny/hosts/<this computer's name>.toml` (the one `bunny install`
wrote; see [Host files](../reference/config.md#host-files)). A shared
`bunny.toml` can have one too; the host file's wins.

## 1. Ask bunny what this computer has

```sh
bunny hardware
```

It changes nothing. It looks at the graphics chips (with `lspci`, or
`/sys/class/drm` when lspci isn't installed) and the processor (with
`cpuid2cpuflags`), and prints a block to paste. On a laptop with Intel and
NVIDIA graphics it looks like this:

```
Graphics found on this computer:
  · Intel: VGA compatible controller [0300]: Intel Corporation … [8086:7d67]
  · NVIDIA: VGA compatible controller [0300]: NVIDIA Corporation … [10de:2c59]

Ready to paste into this computer's host file, /etc/bunny/hosts/laptop.toml:

[hardware]
video_cards = ["intel", "nvidia"]
cpu_flags = ["aes", "avx", "avx2", "f16c", "fma3", "mmx", "mmxext", "pclmul", "popcnt", "rdrand", "sha", "sse", "sse2", "sse3", "sse4_1", "sse4_2", "ssse3"]
```

If it says `cpuid2cpuflags isn't installed`, install it and ask again:

```sh
bunny add app-portage/cpuid2cpuflags
```

```sh
bunny hardware
```

## 2. Paste it into the host file

```sh
sudo nano /etc/bunny/hosts/laptop.toml
```

(Use your computer's name, which `bunny hardware` prints, and `doas` instead
of `sudo` if that's what your system uses.)

**Two graphics chips?** Keep both. On a laptop the built-in one (Intel or
AMD) drives the screen and saves power; the NVIDIA one is used for games and
heavy work.

**CPU features are a trade-off.** They're USE flags, so packages that use
them no longer match Gentoo's prebuilt packages and are compiled on your
computer instead of downloaded. Leave `cpu_flags` out if you'd rather keep
downloading. Graphics cards can mean compiling too, for the packages that
use them (Mesa, mostly).

## 3. Check and switch

```sh
bunny plan
```

It lists the change, like:

```
Graphics cards and CPU features ([hardware]):
  ~ graphics cards (VIDEO_CARDS): not set (Gentoo's defaults) → intel nvidia
```

Then:

```sh
bunny switch
```

`bunny status` counts it among the changes not applied yet until you do.

## Example: an NVIDIA laptop

For NVIDIA's own driver, add `x11-drivers/nvidia-drivers` to the host file
too. A whole host file for an Intel laptop with an NVIDIA card:

```toml
packages = [
    # …what bunny install put here (the kernel, firmware, GRUB) stays…
    "x11-drivers/nvidia-drivers",
]

[pkg."x11-drivers/nvidia-drivers"]
use = ["kernel-open", "dist-kernel"]

[hardware]
video_cards = ["intel", "nvidia"]
cpu_flags = ["aes", "avx", "avx2"]   # paste the full list from `bunny hardware`
```

- **`kernel-open`** builds NVIDIA's open-source kernel module. At the time of
  writing (November 2026), Gentoo's description of the flag on
  [packages.gentoo.org](https://packages.gentoo.org/useflags/kernel-open)
  says it's for Turing (RTX 20-series) and newer cards, that RTX 50-series
  (Blackwell) and newer **need** it, and that drivers from version 595 use
  it whether the flag is on or not. Older cards (GTX 10-series and before)
  leave it out.
- **`dist-kernel`** rebuilds the driver whenever Gentoo's ready-made kernel
  (the one Bunny Linux installs) is updated.

Check the current flags on the package's page,
<https://packages.gentoo.org/packages/x11-drivers/nvidia-drivers>, before you
switch. Gentoo's NVIDIA guide,
<https://wiki.gentoo.org/wiki/NVIDIA/nvidia-drivers>, covers the rest,
including kernel options such as `nvidia_drm.modeset=1`. bunny.toml can't set
kernel options yet (planned), so follow the guide for those.

## Already set up the Gentoo way?

If you followed Gentoo's guides, you may have lines like these in
`/etc/portage/package.use/`:

```
*/* VIDEO_CARDS: -* nvidia
*/* CPU_FLAGS_X86: aes avx avx2 …
```

`bunny import` turns them into a `[hardware]` section. Settings in
`/etc/portage/make.conf` (`VIDEO_CARDS="…"`) are suggested as comments
instead: bunny leaves make.conf alone.

On a switch with `[hardware]`, bunny writes the lines itself (in its own
file, `zz-bunny`) and your old file is moved to the backup. A line that sets
something `[hardware]` doesn't (say `[hardware]` has only `video_cards`, and
you have a CPU features line) is kept word for word in `00-bunny-kept`.

## What bunny writes

```
*/* VIDEO_CARDS: -* intel nvidia
*/* CPU_FLAGS_X86: aes avx avx2
```

`video_cards` is the whole list (`-*` turns off Gentoo's defaults first, as
Gentoo's NVIDIA guide does). `cpu_flags` adds to Gentoo's defaults, the way
`cpuid2cpuflags` output does. The exact rules are in
[`[hardware]`](../reference/config.md#hardware).
