"""bunny's Portage helper: asks Portage itself and answers in JSON.

bunny used to read emerge's screen text, which breaks whenever emerge changes
its wording. This script uses Portage's own library (the code emerge runs)
and prints exact data instead. It's shipped inside bunny and run with the
system's python3, which has Portage.

Usage (one JSON object on stdout; anything Portage prints goes to stderr;
exit status 0 when Portage answered, even with "no plan"):

    python3 bunny-portage.py plan [atom ...]
    python3 bunny-portage.py depclean

    plan: what `emerge --update --deep --newuse <atoms>` would install
          (default atom: @world), with the same options emerge would use
          (EMERGE_DEFAULT_OPTS and FEATURES=getbinpkg included).
          {"ok": true, "merges": [{"name", "version", "from", "action",
                                    "binary", "slot", "why"}]}
          "why" (rebuilds only): {"use": ["+flag", "-flag"], "libraries":
          ["cat/pkg"]}: the USE flags that changed, and the libraries it
          uses whose sub-slot changes in this plan (emerge's `r`).
          {"ok": false, "reason": "...", "problems": [...]} when Portage
          can't work out a plan. bunny then still asks emerge (for the log
          and for kinds of problem the helper doesn't report yet), and
          explains the problems below from this data instead of emerge's
          text. Each problem has a "kind":
            {"kind": "keywords", "name", "version", "keyword",
             "required_by"}: only a testing (or live, "**") version would
             do; `keyword` is what to accept.
            {"kind": "masked", "name", "atom", "reasons", "comment",
             "required_by"}: every version that could do is masked;
             `reasons` like "package.mask", "~amd64 keyword", "missing
             keyword"; `comment` is Gentoo's package.mask comment, if any.
            {"kind": "use", "name", "version", "changes", "required_by"}:
             it must be built with other USE flags; `changes` maps each
             flag to true (on) or false (off).
          `required_by`: the packages that pull it in, nearest first (may
          be empty). Kinds bunny doesn't know are ignored.

    resolve [--usepkg-exclude=a,b] [atom ...]: what emerge needs changed to
          work out a plan for `--update --deep --newuse <atoms>` (default
          @world), found the way a Gentoo user would by hand, but in one go.
          Works on a scratch copy of /etc/portage (PORTAGE_CONFIGROOT);
          nothing on the system changes. Each round asks emerge (--pretend,
          --autounmask) and applies what stops it:
            - USE changes emerge says are needed;
            - testing-only versions (keywords);
            - "any-of ( a b )" REQUIRED_USE and emerge's "One of the
              following ... (Change USE: ...)": picks one (Wayland first,
              then flags turned on rather than off);
            - a flag already chosen (by your config or an earlier round) is
              never flipped back: the package that pins it the other way
              (usually a prebuilt copy built with other flags) gets the
              same flag instead, so a matching prebuilt flavour is used;
              if that doesn't help, it's compiled instead (usepkg-exclude).
          {"ok": true, "resolved": bool, "use": {cp: {flag: on}},
           "keywords": {cp: kw}, "compile": [cp], "why": {cp: "..."},
           "rounds": n, "stuck": "end of emerge's last message"}
          Only changes beyond the config are listed.


    depclean: what `emerge --depclean` would remove (nothing is removed),
          with EMERGE_DEFAULT_OPTS.
          {"ok": true, "removals": [{"name", "version", "slot",
                                      "needed_by"}]}
          `needed_by`: other packages in the same list whose recorded
          dependencies name this one (so it goes because they go). Portage
          doesn't record why anything else is unneeded: nothing installed
          that's being kept depends on it, which is what depclean means.
          {"ok": false, "reason": "..."} when Portage won't say (for example
          a dependency problem); bunny then asks emerge itself.

Licence: GPL-3.0 (part of bunny).
"""

import json
import os
import shlex
import sys

HELPER_VERSION = 4


def say(obj):
    sys.__stdout__.write(json.dumps(obj) + "\n")
    sys.__stdout__.flush()


def plan(atoms):
    # Portage prints progress ("Calculating dependencies...") to stdout;
    # keep stdout for our JSON only.
    sys.stdout = sys.stderr

    import portage
    from portage.versions import vercmp
    from _emerge.actions import load_emerge_config
    from _emerge.create_depgraph_params import create_depgraph_params
    from _emerge.depgraph import backtrack_depgraph
    from _emerge.main import parse_opts
    from _emerge.Package import Package
    from _emerge.stdout_spinner import stdout_spinner

    # Portage's configuration as emerge sees it: make.conf, profiles,
    # package.use and friends (`settings`), and the package databases
    # (`trees`): the repositories, what's installed (vartree) and prebuilt
    # packages (bintree), for each root.
    config = load_emerge_config()
    settings = config.target_config.settings
    trees = config.trees

    # The same options a plain `emerge` run would get.
    args = shlex.split(settings.get("EMERGE_DEFAULT_OPTS", ""))
    # Options bunny adds (like --usepkg-exclude=a,b for packages the config
    # says to compile) come before the atoms.
    options = []
    for a in atoms:
        if a.startswith("--usepkg-exclude="):
            options.append("--usepkg-exclude=" + " ".join(a.split("=", 1)[1].split(",")))
    atoms = [a for a in atoms if not a.startswith("--")]
    args += ["--pretend", "--update", "--deep", "--newuse"] + options + (atoms or ["@world"])
    # emerge's own command-line parser, so options mean exactly what they
    # mean to emerge (including anything from EMERGE_DEFAULT_OPTS).
    _action, opts, files = parse_opts(args, silent=True)
    # emerge turns FEATURES=getbinpkg into --getbinpkg, and --getbinpkg into
    # --usepkg, before planning; parse_opts alone doesn't. Same here, so a
    # system set up for prebuilt packages gets the same plan as emerge.
    if "getbinpkg" in settings.features:
        opts["--getbinpkg"] = True
    if "--getbinpkg" in opts:
        opts["--usepkg"] = True
    if "--usepkg" in opts:
        # Read the list of prebuilt packages (downloading the binhost's index
        # with --getbinpkg), as emerge does before planning.
        for root_trees in trees.values():
            try:
                root_trees["bintree"].populate(getbinpkgs="--getbinpkg" in opts)
            except Exception as e:  # an unreachable binhost: plan without it
                sys.stderr.write("bunny-portage: prebuilt packages unavailable: %s\n" % e)

    # No spinner animation: our output is JSON. (Don't rely on the spinner's
    # method names; they change between Portage versions.)
    spinner = stdout_spinner()
    spinner.update = lambda *args, **kwargs: True
    # The dependency calculation itself, the same call emerge makes: it
    # resolves `files` (the atoms) and retries with backtracking when it
    # hits a conflict. `ok` is false when no plan works; emerge would print
    # why, so bunny then runs emerge to explain it.
    params = create_depgraph_params(opts, "")
    ok, graph, _favorites = backtrack_depgraph(settings, trees, opts, params, "", files, spinner)
    if not ok:
        try:
            found = problems(graph, settings, trees)
        except Exception as e:  # Portage's insides changed: emerge's text will do
            sys.stderr.write("bunny-portage: couldn't read the problems: %s\n" % e)
            found = []
        return {"ok": False, "reason": "portage couldn't work out a plan", "problems": found}

    # What's installed, to tell new packages from updates and rebuilds.
    vardb = trees[settings["EROOT"]]["vartree"].dbapi
    tasks = [t for t in graph.altlist() if isinstance(t, Package) and t.operation == "merge"]
    # The sub-slot each package in the plan will have, for "a library it
    # uses changed" (Gentoo's slot operator rebuilds).
    new_sub_slots = {t.cp: t.sub_slot for t in tasks}
    merges = []
    # The merge list in install order, as `emerge --pretend` would print it.
    # It also holds other kinds of tasks (uninstalls, blockers); only
    # packages being merged are kept.
    for task in tasks:
        # The installed version in the same slot, if any (`cat/pkg:slot`).
        installed = vardb.match(task.slot_atom)
        if not installed:
            action, old = "new", None
        else:
            old = portage.versions.cpv_getversion(installed[-1])
            diff = vercmp(task.version, old)
            action = "update" if diff > 0 else "downgrade" if diff < 0 else "rebuild"
        entry = {}
        if action == "rebuild":
            entry["why"] = why_rebuild(task, installed[-1], vardb, new_sub_slots)
        merges.append(
            {
                "name": task.cp,
                "version": task.version,
                "from": old,
                "action": action,
                "binary": task.type_name == "binary",
                "slot": task.slot,
                **entry,
            }
        )
    return {"ok": True, "merges": merges}


def problems(graph, settings, trees):
    """Why a plan failed, for the kinds bunny explains from data.

    These come from the depgraph's own records: what it would have to
    change to work (emerge's "autounmask" suggestions) and the dependencies
    it couldn't meet. They're internal to Portage (names starting with _),
    so each is read defensively: anything missing just means bunny falls
    back to emerge's text for that kind.
    """
    import portage

    dynamic = graph._dynamic_config
    portdb = trees[settings["EROOT"]]["porttree"].dbapi
    arch = settings.get("ARCH", "")
    found = []

    def required_by(pkg):
        # The packages that pulled this one in, nearest first: walk up the
        # dependency graph through the first parent that's a package.
        chain = []
        node = pkg
        try:
            for _ in range(8):
                parents = [p for p in dynamic.digraph.parent_nodes(node) if hasattr(p, "cp")]
                if not parents or parents[0].cp in chain or parents[0].cp == pkg.cp:
                    break
                node = parents[0]
                chain.append(node.cp)
        except Exception:
            pass
        return chain

    # Only a testing version would do: emerge's "The following keyword
    # changes are necessary". "~arch" when the version is in testing for
    # this architecture; "**" (any keyword at all) for live versions and
    # ones with no keyword for it.
    for pkg in sorted(getattr(dynamic, "_needed_unstable_keywords", ()), key=lambda p: p.cpv):
        try:
            keywords = portdb.aux_get(pkg.cpv, ["KEYWORDS"], myrepo=pkg.repo)[0].split()
        except Exception:
            keywords = []
        keyword = "~" + arch if arch and "~" + arch in keywords else "**"
        found.append(
            {
                "kind": "keywords",
                "name": pkg.cp,
                "version": pkg.version,
                "keyword": keyword,
                "required_by": required_by(pkg),
            }
        )

    # Masked on purpose by Gentoo (package.mask), when emerge would unmask it.
    for pkg in sorted(getattr(dynamic, "_needed_p_mask_changes", ()), key=lambda p: p.cpv):
        found.append(masked_problem(pkg.cp, str(pkg.cpv), [pkg.cpv], settings, portdb, required_by(pkg)))

    # Built with other USE flags: emerge's "The following USE changes are
    # necessary". {package: (the flags it would have, {flag: on/off})}.
    for pkg, value in sorted(
        getattr(dynamic, "_needed_use_config_changes", {}).items(), key=lambda kv: kv[0].cpv
    ):
        changes = value[1] if isinstance(value, tuple) and len(value) > 1 else value
        if not isinstance(changes, dict) or not changes:
            continue
        found.append(
            {
                "kind": "use",
                "name": pkg.cp,
                "version": pkg.version,
                "changes": {flag: bool(on) for flag, on in sorted(changes.items())},
                "required_by": required_by(pkg),
            }
        )

    # Dependencies it couldn't meet at all: emerge's "All ebuilds that could
    # satisfy ... have been masked". Only when every version is masked; the
    # other reasons (no such package, USE requirements) stay with emerge's
    # text for now.
    for item in getattr(dynamic, "_unsatisfied_deps_for_display", ()):
        try:
            (_root, atom), _kwargs = item
            plain = atom.without_use if hasattr(atom, "without_use") else atom
            cpvs = portdb.xmatch("match-all", plain)
        except Exception:
            continue
        if not cpvs:
            continue
        reasons = {cpv: portage.getmaskingstatus(cpv, settings=settings, portdb=portdb) for cpv in cpvs}
        if not all(reasons.values()):
            continue  # a version isn't masked: the problem is something else
        found.append(masked_problem(atom.cp, str(atom), cpvs, settings, portdb, [], reasons))
    return found


def masked_problem(cp, atom, cpvs, settings, portdb, chain, reasons=None):
    import portage

    if reasons is None:
        reasons = {cpv: portage.getmaskingstatus(cpv, settings=settings, portdb=portdb) for cpv in cpvs}
    every = sorted({r for rs in reasons.values() for r in rs})
    comment = None
    if "package.mask" in every:
        for cpv in cpvs:
            try:
                text = portage.getmaskingreason(cpv, settings=settings, portdb=portdb)
            except Exception:
                text = None
            if text:
                # "# Author <email> (date)" first, then the reason.
                lines = [l.lstrip("#").strip() for l in text.splitlines() if l.startswith("#")]
                comment = " ".join(l for l in lines[1:] if l) or None
                break
    return {
        "kind": "masked",
        "name": cp,
        "atom": atom,
        "reasons": every,
        "comment": comment,
        "required_by": chain,
    }


def why_rebuild(task, installed_cpv, vardb, new_sub_slots):
    """Why the same version is installed again: USE flags that changed, and
    libraries it was built against whose sub-slot changes in this plan.
    Never fails the plan: an empty answer just means "(rebuild)"."""
    why = {"use": [], "libraries": []}
    try:
        old_use, old_iuse = vardb.aux_get(installed_cpv, ["USE", "IUSE"])
        old_enabled = set(old_use.split())
        old_iuse = {f.lstrip("+-") for f in old_iuse.split()}
        new_enabled = set(task.use.enabled)
        new_iuse = set(task.iuse.all)
        for flag in sorted(new_iuse | old_iuse):
            was = flag in old_iuse and flag in old_enabled
            now = flag in new_iuse and flag in new_enabled
            if was != now:
                why["use"].append(("+" if now else "-") + flag)
    except Exception:
        pass
    try:
        from portage.dep import Atom

        # The installed package's dependencies, with the sub-slot each
        # library had when it was built (`dev-libs/openssl:0/3=`).
        deps = " ".join(vardb.aux_get(installed_cpv, ["DEPEND", "RDEPEND", "BDEPEND", "PDEPEND"]))
        seen = set()
        for token in deps.split():
            try:
                atom = Atom(token)
            except Exception:
                continue  # "||", "(", "flag?" and the like
            if atom.slot_operator != "=" or atom.sub_slot is None or atom.cp in seen:
                continue
            new = new_sub_slots.get(atom.cp)
            if new is not None and new != atom.sub_slot:
                seen.add(atom.cp)
                why["libraries"].append(atom.cp)
    except Exception:
        pass
    return why


def depclean():
    # As in plan(): stdout is for our JSON only.
    sys.stdout = sys.stderr

    import portage
    from portage.dep import Atom, use_reduce
    from portage.versions import cpv_getkey, cpv_getversion
    from _emerge.actions import calc_depclean, load_emerge_config
    from _emerge.main import parse_opts
    from _emerge.stdout_spinner import stdout_spinner

    from portage._sets.base import InternalPackageSet

    config = load_emerge_config()
    settings = config.target_config.settings
    trees = config.trees

    # The options a plain `emerge --depclean --pretend` would get.
    args = shlex.split(settings.get("EMERGE_DEFAULT_OPTS", ""))
    args += ["--depclean", "--pretend"]
    _action, opts, _files = parse_opts(args, silent=True)

    spinner = stdout_spinner()
    spinner.update = lambda *args, **kwargs: True

    # The calculation `emerge --depclean` makes before removing anything:
    # everything installed that @world (and @system, and anything a package
    # still in them needs) doesn't reach. No packages given, so the whole
    # system is checked, exactly like a plain `emerge --depclean`.
    # ldpath_mtimes is only used when removing; nothing is removed here.
    args_set = InternalPackageSet(allow_repo=True)
    result = calc_depclean(
        settings, trees, config.target_config.mtimedb["ldpath"], opts, "depclean", args_set, spinner
    )
    # (returncode, cleanlist, ordered, req_pkg_count, unmerge_deps) in
    # Portage 3.0.7x; older versions return fewer fields. Only the first two
    # are used.
    returncode, cleanlist = result[0], result[1]
    if returncode != 0:
        return {"ok": False, "reason": "portage couldn't work out what to remove"}

    vardb = trees[settings["EROOT"]]["vartree"].dbapi
    # The list holds package versions (`cat/pkg-1.0`), or Package objects in
    # some versions.
    cpvs = [str(getattr(item, "cpv", item)) for item in cleanlist]
    names = {cpv: cpv_getkey(cpv) for cpv in cpvs}
    going = set(names.values())

    def needs(cpv):
        # Package names this installed package's recorded dependencies
        # mention, with its own USE flags applied.
        keys = ["RDEPEND", "PDEPEND", "DEPEND", "BDEPEND", "IDEPEND", "USE", "EAPI"]
        try:
            values = dict(zip(keys, vardb.aux_get(cpv, keys)))
        except Exception:
            return set()
        use = set(values["USE"].split())
        found = set()
        for key in keys[:5]:
            try:
                tokens = use_reduce(
                    values[key], uselist=use, flat=True, eapi=values["EAPI"] or None
                )
            except Exception:
                continue
            for token in tokens:
                try:
                    atom = Atom(token)
                except Exception:  # "||" and other markers
                    continue
                if not atom.blocker:
                    found.add(atom.cp)
        return found

    needed = {cpv: needs(cpv) & going for cpv in cpvs}
    removals = []
    for cpv in cpvs:
        try:
            slot = vardb.aux_get(cpv, ["SLOT"])[0].split("/")[0]
        except Exception:
            slot = None
        removals.append(
            {
                "name": names[cpv],
                "version": cpv_getversion(cpv),
                "slot": slot,
                "needed_by": sorted(
                    names[other]
                    for other in cpvs
                    if other != cpv and names[cpv] in needed[other] and names[other] != names[cpv]
                ),
            }
        )
    return {"ok": True, "removals": removals}


# --- resolve -------------------------------------------------------------

PREFERRED = ["wayland", "X", "opengl", "vulkan", "gtk", "qt6", "elogind", "dbus"]
MAX_ROUNDS = 30


def _cp_of(atom):
    """`>=cat/pkg-1.2-r3:slot[flags]` -> `cat/pkg`."""
    import re

    atom = re.sub(r"^[<>=~!]+", "", atom.split("[")[0].split("::")[0])
    atom = atom.split(":")[0]
    m = re.match(r"([A-Za-z0-9][\w+.-]*/[\w+.-]+?)(-\d[\w.]*(?:-r\d+)?)?$", atom)
    return m.group(1) if m else atom


def _section(out, title):
    """`cat/pkg flags` lines under one of emerge's autounmask headings."""
    import re

    found = []
    if title not in out:
        return found
    for line in out.split(title, 1)[1].splitlines()[1:]:
        line = line.strip()
        if not line or line.startswith("(") or line.startswith("#"):
            continue
        if not re.match(r"^[<>=~]*[A-Za-z0-9][\w+.-]*/", line):
            break
        found.append(line)
    return found


def _required_by(out, line):
    """The nearest `# required by cat/pkg` above an autounmask line."""
    import re

    m = re.search(r"# required by (\S+?)(?:::\S+)?\n(?:#.*\n)*" + re.escape(line), out)
    return _cp_of(m.group(1)) if m else None


def _required_use(out):
    """[(cp, [flags])] for unmet any-of / exactly-one-of REQUIRED_USE."""
    import re

    found = []
    for m in re.finditer(
        r"has unmet requirements\.\n- (\S+?)::\S+ USE=\"[^\"]*\".*?\n\n"
        r"  The following REQUIRED_USE flag constraints are unsatisfied:\n((?:    .*\n)+)",
        out,
    ):
        for line in m.group(2).splitlines():
            g = re.search(r"(?:any-of|\^\^|\|\|) \( ([^)]*) \)", line)
            if g:
                found.append((_cp_of(m.group(1)), g.group(1).split()))
    return found


def _one_of(out):
    """emerge's "One of the following packages is required" with
    "(Change USE: ...)" options: [(cp, [(flag, on)], parent)], the option
    that only turns flags on first, and the package that asked."""
    import re

    found = []
    pattern = (
        r"!!! One of the following packages is required to complete your request:\n"
        r"((?:- .*\n)+)\(dependency required by \"(\S+?)(?:::\S+?)?\" \[(\w+)\]\)"
    )
    for m in re.finditer(pattern, out):
        options = []
        for line in m.group(1).splitlines():
            g = re.match(r"- (\S+?)::\S+ \(Change USE: ([^)]*)\)", line)
            if g:
                changes = [(f.lstrip("+-"), not f.startswith("-")) for f in g.group(2).split()]
                options.append((_cp_of(g.group(1)), changes))
        options.sort(key=lambda o: not all(on for _, on in o[1]))
        if options:
            found.append((options[0][0], options[0][1], _cp_of(m.group(2))))
    return found


def _read_flags(path):
    """{cp: {flag: on}} from a package.use file bunny wrote."""
    flags = {}
    try:
        text = open(path).read()
    except OSError:
        return flags
    for line in text.splitlines():
        parts = line.split("#", 1)[0].split()
        if len(parts) < 2 or "/" not in parts[0] or parts[0].startswith("*"):
            continue
        cp = _cp_of(parts[0])
        for f in parts[1:]:
            if f.endswith(":"):
                break  # USE_EXPAND (VIDEO_CARDS: ...) isn't a plain flag
            flags.setdefault(cp, {})[f.lstrip("+-")] = not f.startswith("-")
    return flags


def resolve(args):
    import re
    import shutil
    import subprocess
    import tempfile

    sys.stdout = sys.stderr
    exclude = set()
    atoms = []
    for a in args:
        if a.startswith("--usepkg-exclude="):
            exclude.update(x for x in a.split("=", 1)[1].split(",") if x)
        else:
            atoms.append(a)
    atoms = atoms or ["@world"]

    config_root = os.environ.get("PORTAGE_CONFIGROOT", "/")
    real = os.path.join(config_root, "etc/portage")
    scratch = tempfile.mkdtemp(prefix="bunny-resolve-")
    try:
        shutil.copytree(real, os.path.join(scratch, "etc/portage"), symlinks=True)
        profile = os.path.join(scratch, "etc/portage/make.profile")
        if os.path.islink(profile):
            target = os.path.realpath(os.path.join(real, "make.profile"))
            os.remove(profile)
            os.symlink(target, profile)
        use_dir = os.path.join(scratch, "etc/portage/package.use")
        kw_dir = os.path.join(scratch, "etc/portage/package.accept_keywords")
        for d in (use_dir, kw_dir):
            if os.path.isfile(d):  # a single file instead of a folder
                text = open(d).read()
                os.remove(d)
                os.makedirs(d)
                open(os.path.join(d, "00-file"), "w").write(text)
            os.makedirs(d, exist_ok=True)

        chosen = _read_flags(os.path.join(use_dir, "zz-bunny"))  # the config's own
        given = {cp: dict(fl) for cp, fl in chosen.items()}
        keywords = {}
        why = {}
        status, out, rounds = 1, "", 0

        def write():
            with open(os.path.join(use_dir, "zzz-bunny-resolve"), "w") as f:
                for cp, fl in sorted(chosen.items()):
                    f.write(cp + " " + " ".join(("" if on else "-") + n for n, on in sorted(fl.items())) + "\n")
            with open(os.path.join(kw_dir, "zzz-bunny-resolve"), "w") as f:
                for cp, kw in sorted(keywords.items()):
                    f.write(cp + " " + kw + "\n")

        def choose(cp, name, on, reason):
            if chosen.get(cp, {}).get(name) == on:
                return False
            chosen.setdefault(cp, {})[name] = on
            why.setdefault(cp, reason)
            return True

        while rounds < MAX_ROUNDS:
            rounds += 1
            write()
            cmd = ["emerge", "--color=n", "--ask=n", "--pretend", "--verbose", "--update",
                   "--deep", "--newuse", "--autounmask=y", "--autounmask-write=n",
                   "--autounmask-backtrack=y"]
            if exclude:
                cmd.append("--usepkg-exclude=" + " ".join(sorted(exclude)))
            env = dict(os.environ, PORTAGE_CONFIGROOT=scratch)
            p = subprocess.run(cmd + atoms, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, universal_newlines=True)
            status, out = p.returncode, p.stdout
            sys.stderr.write("bunny-resolve: round %d\n" % rounds)
            sys.stderr.flush()
            if status == 0:
                break
            changed = False
            for line in _section(out, "The following USE changes are necessary to proceed:"):
                parts = line.split()
                cp = _cp_of(parts[0])
                parent = _required_by(out, line)
                for f in parts[1:]:
                    name, on = f.lstrip("+-"), not f.startswith("-")
                    have = chosen.get(cp, {}).get(name)
                    if have is None:
                        changed |= choose(cp, name, on, "needed by %s" % (parent or "what you asked for"))
                    elif have != on and parent:
                        # Something pins this flag the other way: give it
                        # the same flag, or compile it if that was tried.
                        if chosen.get(parent, {}).get(name) != have:
                            changed |= choose(parent, name, have, "to match %s" % cp)
                        elif parent not in exclude:
                            exclude.add(parent)
                            why.setdefault(parent, "its prebuilt copy pins %s" % cp)
                            changed = True
            for line in _section(out, "The following keyword changes are necessary to proceed:"):
                parts = line.split()
                cp = _cp_of(parts[0])
                if cp not in keywords:
                    keywords[cp] = parts[1] if len(parts) > 1 else "~" + os.environ.get("ARCH", "amd64")
                    why.setdefault(cp, "only a testing version, needed by %s" % (_required_by(out, line) or "what you asked for"))
                    changed = True
            for cp, options in _required_use(out):
                pick = next((f for f in PREFERRED if f in options), options[0])
                if not any(chosen.get(cp, {}).get(f) for f in options):
                    changed |= choose(cp, pick, True, "needs one of %s" % " ".join(options))
            for cp, changes, parent in _one_of(out):
                clash = [(n, on) for n, on in changes if chosen.get(cp, {}).get(n) not in (None, on)]
                if clash:
                    for n, _ in clash:
                        want = chosen[cp][n]
                        if chosen.get(parent, {}).get(n) != want:
                            changed |= choose(parent, n, want, "to match %s" % cp)
                        elif parent not in exclude:
                            exclude.add(parent)
                            why.setdefault(parent, "its prebuilt copy pins %s" % cp)
                            changed = True
                    continue
                for n, on in changes:
                    if chosen.get(cp, {}).get(n) is None:
                        changed |= choose(cp, n, on, "needed by %s" % parent)
            if not changed:
                break

        # Only what goes beyond the config itself.
        use = {}
        for cp, fl in sorted(chosen.items()):
            extra = {n: on for n, on in fl.items() if given.get(cp, {}).get(n) != on}
            if extra:
                use[cp] = extra
        result = {
            "ok": True,
            "resolved": status == 0,
            "use": use,
            "keywords": keywords,
            "compile": sorted(exclude),
            "why": why,
            "rounds": rounds,
        }
        if status != 0:
            result["stuck"] = "\n".join(out.rstrip().splitlines()[-40:])
        return result
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main(argv):
    if len(argv) < 2:
        say({"ok": False, "reason": "usage: bunny-portage.py plan [atom ...] | depclean | resolve [atom ...]"})
        return 2
    command, rest = argv[1], argv[2:]
    try:
        if command == "version":
            result = {"ok": True, "version": HELPER_VERSION}
        elif command == "plan":
            result = plan(rest)
        elif command == "depclean":
            result = depclean()
        elif command == "resolve":
            result = resolve(rest)
        else:
            result = {"ok": False, "reason": "unknown command: %s" % command}
    except Exception as e:  # never crash with a traceback on stdout
        result = {"ok": False, "reason": "%s: %s" % (type(e).__name__, e)}
    result["helper"] = HELPER_VERSION
    say(result)
    # "No plan, and here's why" is an answer too: exit 0 so bunny reads it.
    return 0 if result.get("ok") or "problems" in result else 1


if __name__ == "__main__":
    os.environ.setdefault("PORTAGE_QUIET", "1")
    sys.exit(main(sys.argv))
