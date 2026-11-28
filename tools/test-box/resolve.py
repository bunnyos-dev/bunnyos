"""Prototype of bunny's resolver, run in the lab against the real Portage.

Keeps asking emerge (--pretend) for a plan and fixes what stops it, the way a
Gentoo user would by hand, until there's a plan or nothing more can be done:

  * USE changes emerge says are needed: applied per package, but a flag
    bunny already chose is never flipped back; when emerge asks for that,
    the package that pins the flag (its parent) gets the same flag instead,
    so a matching prebuilt flavour can be used.
  * Testing-only versions: accepted for that package.
  * "any-of ( a b )" REQUIRED_USE: picks the first of the preferred flags
    that's in the list (wayland before X for a Wayland desktop).
  * Slot conflicts between prebuilt packages built at different times:
    the package that pins the *older* version is compiled instead
    (--usepkg-exclude), so it builds against the newer one.

Usage: python3 resolve.py <portage config root> <atom> ...
Writes its changes into <root>/etc/portage/{package.use,package.accept_keywords}/zz-resolve
and prints a JSON summary at the end.
"""

import json
import os
import re
import subprocess
import sys

PREFERRED = ["wayland", "X", "opengl", "vulkan", "gtk", "qt6", "elogind", "dbus"]
MAX_ROUNDS = 25


def emerge(root, atoms, exclude):
    cmd = ["emerge", "--color=n", "--ask=n", "--pretend", "--verbose", "--autounmask=y",
           "--autounmask-write=n", "--autounmask-backtrack=y"]
    if exclude:
        cmd.append("--usepkg-exclude=" + " ".join(sorted(exclude)))
    env = dict(os.environ, PORTAGE_CONFIGROOT=root)
    p = subprocess.run(cmd + atoms, env=env, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def cp_of(atom):
    """`>=cat/pkg-1.2-r3` -> `cat/pkg`."""
    atom = re.sub(r"^[<>=~!]+", "", atom.split(":")[0].split("[")[0])
    m = re.match(r"([a-z0-9][\w+.-]*/[\w+.-]+?)(-\d[\w.]*(?:-r\d+)?)?$", atom)
    return m.group(1) if m else atom


def blocks(out, title):
    """Lines of `cat/pkg flags` under an autounmask section."""
    found = []
    if title not in out:
        return found
    part = out.split(title, 1)[1]
    for line in part.splitlines()[1:]:
        line = line.strip()
        if not line or line.startswith("("):
            continue
        if line.startswith("#"):
            continue
        if not re.match(r"^[<>=~]*[a-z0-9][\w+.-]*/", line):
            break
        found.append(line)
    return found


def required_use_choices(out):
    """[(cp, [flags])] for unsatisfied any-of/^^ REQUIRED_USE blocks."""
    found = []
    for m in re.finditer(
        r'has unmet requirements\.\n- (\S+?)::\S+ USE="[^"]*".*?\n\n  The following REQUIRED_USE flag constraints are unsatisfied:\n((?:    .*\n)+)',
        out,
    ):
        cp = cp_of(m.group(1))
        for line in m.group(2).splitlines():
            g = re.search(r"(any-of|\^\^|\|\|) \( ([^)]*) \)", line)
            if g:
                found.append((cp, g.group(2).split()))
    return found


def one_of_use(out):
    """emerge's "One of the following packages is required" with
    "(Change USE: ...)" options. Returns [(cp, [(flag, on)], parent)] for
    each such block: the options in emerge's order (ones that only turn
    flags on first), and the package that asked (its nearest
    "dependency required by"), which is what pins the flags when an option
    would undo a choice already made."""
    found = []
    pattern = (r"!!! One of the following packages is required to complete your request:\n"
               r"((?:- .*\n)+)\(dependency required by \"(\S+?)::\S+?\" \[(\w+)\]\)")
    for m in re.finditer(pattern, out):
        options = []
        for line in m.group(1).splitlines():
            g = re.match(r"- (\S+?)::\S+ \(Change USE: ([^)]*)\)", line)
            if g:
                changes = [(f.lstrip("+-"), not f.startswith("-")) for f in g.group(2).split()]
                options.append((cp_of(g.group(1)), changes))
        options.sort(key=lambda o: not all(on for _, on in o[1]))
        for cp, changes in options:
            found.append((cp, changes, cp_of(m.group(2))))
            break
    return found


def slot_conflicts(out):
    """Parents that pin an older version of a package, both binaries.

    Lines look like:
      dev-util/glslang:0/16.4= required by (dev-util/shader-slang-..., binary scheduled for merge)
    Returns parents (cp) holding the lowest sub-slot/version per conflict.
    """
    if "slot conflict" not in out:
        return []
    section = out.split("slot conflict", 1)[1].split("\n\n\n", 1)[0]
    pins = {}
    for m in re.finditer(r"^\s+(\S+?)(?:\[.*?\])? required by \((\S+?)::\S+?, (binary|ebuild|installed)", section, re.M):
        dep, parent, kind = m.group(1), m.group(2), m.group(3)
        if kind != "binary":
            continue
        pins.setdefault(cp_of(dep), []).append((dep, cp_of(parent.rsplit(":", 1)[0])))
    olds = []
    for cp, items in pins.items():
        if len(items) < 2:
            continue

        def key(item):
            nums = re.findall(r"\d+", item[0].split(cp, 1)[-1])
            return [int(n) for n in nums]

        items.sort(key=key)
        olds.append(items[0][1])
    return olds


def main():
    root = sys.argv[1]
    atoms = sys.argv[2:]
    use_dir = os.path.join(root, "etc/portage/package.use")
    kw_dir = os.path.join(root, "etc/portage/package.accept_keywords")
    os.makedirs(use_dir, exist_ok=True)
    os.makedirs(kw_dir, exist_ok=True)
    flags = {}  # cp -> {flag: bool}
    keywords = {}  # cp -> keyword
    exclude = set()
    log = []

    def write():
        with open(os.path.join(use_dir, "zz-resolve"), "w") as f:
            for cp, fl in sorted(flags.items()):
                f.write(cp + " " + " ".join(("" if on else "-") + n for n, on in sorted(fl.items())) + "\n")
        with open(os.path.join(kw_dir, "zz-resolve"), "w") as f:
            for cp, kw in sorted(keywords.items()):
                f.write(cp + " " + kw + "\n")

    status, out = 1, ""
    for round_no in range(1, MAX_ROUNDS + 1):
        write()
        status, out = emerge(root, atoms, exclude)
        if status == 0:
            break
        changed = []
        for line in blocks(out, "The following USE changes are necessary to proceed:"):
            parts = line.split()
            cp = cp_of(parts[0])
            for f in parts[1:]:
                on = not f.startswith("-")
                name = f.lstrip("-+")
                have = flags.get(cp, {}).get(name)
                if have is None:
                    flags.setdefault(cp, {})[name] = on
                    changed.append("%s %s%s" % (cp, "" if on else "-", name))
                elif have != on:
                    # Contradiction: something pins this flag the other way.
                    # Find who: the "# required by" line just above it.
                    m = re.search(r"# required by (\S+?)::\S+\n(?:#.*\n)*" + re.escape(line), out)
                    parent = cp_of(m.group(1)) if m else None
                    if parent and flags.get(parent, {}).get(name) != have:
                        flags.setdefault(parent, {})[name] = have
                        changed.append("%s %s%s (pins %s)" % (parent, "" if have else "-", name, cp))
                    elif parent and parent not in exclude:
                        exclude.add(parent)
                        changed.append("compile %s (its prebuilt copy pins %s %s)" % (parent, cp, name))
        for line in blocks(out, "The following keyword changes are necessary to proceed:"):
            parts = line.split()
            cp = cp_of(parts[0])
            if cp not in keywords:
                keywords[cp] = parts[1] if len(parts) > 1 else "~amd64"
                changed.append("%s %s" % (cp, keywords[cp]))
        for cp, options in required_use_choices(out):
            pick = next((p for p in PREFERRED if p in options), options[0])
            if flags.get(cp, {}).get(pick) is not True:
                flags.setdefault(cp, {})[pick] = True
                changed.append("%s %s (one of %s)" % (cp, pick, " ".join(options)))
        for cp, changes, parent in one_of_use(out):
            clash = [(n, on) for n, on in changes if flags.get(cp, {}).get(n) not in (None, on)]
            if clash:
                # The parent pins these flags the other way (a prebuilt
                # copy built without them): give the parent the flags
                # already chosen, so a matching prebuilt flavour (or a
                # build) is used instead of flipping them back.
                for n, _ in clash:
                    want = flags[cp][n]
                    if flags.get(parent, {}).get(n) != want:
                        flags.setdefault(parent, {})[n] = want
                        changed.append("%s %s%s (to match %s)" % (parent, "" if want else "-", n, cp))
                continue
            for n, on in changes:
                if flags.get(cp, {}).get(n) is None:
                    flags.setdefault(cp, {})[n] = on
                    changed.append("%s %s%s (emerge offered a choice)" % (cp, "" if on else "-", n))
        for parent in slot_conflicts(out):
            if parent not in exclude:
                exclude.add(parent)
                changed.append("compile %s (its prebuilt copy was built against an older version)" % parent)
        log.append({"round": round_no, "changes": changed})
        print("round %d: %s" % (round_no, "; ".join(changed) or "nothing new"), flush=True)
        if not changed:
            break

    binaries = len(re.findall(r"^\[binary", out, re.M))
    builds = re.findall(r"^\[ebuild[^\]]*\] (\S+)", out, re.M)
    summary = {
        "resolved": status == 0,
        "rounds": len(log),
        "prebuilt": binaries,
        "compile": builds,
        "use": {cp: fl for cp, fl in sorted(flags.items())},
        "keywords": keywords,
        "compile_instead_of_prebuilt": sorted(exclude),
    }
    if status != 0:
        summary["stuck_on"] = out[-3000:]
    print(json.dumps(summary, indent=1))


if __name__ == "__main__":
    main()
