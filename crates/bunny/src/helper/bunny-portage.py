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
                                    "binary", "slot"}]}
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

HELPER_VERSION = 3


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
    args += ["--pretend", "--update", "--deep", "--newuse"] + (atoms or ["@world"])
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
    merges = []
    # The merge list in install order, as `emerge --pretend` would print it.
    # It also holds other kinds of tasks (uninstalls, blockers); only
    # packages being merged are kept.
    for task in graph.altlist():
        if not isinstance(task, Package) or task.operation != "merge":
            continue
        # The installed version in the same slot, if any (`cat/pkg:slot`).
        installed = vardb.match(task.slot_atom)
        if not installed:
            action, old = "new", None
        else:
            old = portage.versions.cpv_getversion(installed[-1])
            diff = vercmp(task.version, old)
            action = "update" if diff > 0 else "downgrade" if diff < 0 else "rebuild"
        merges.append(
            {
                "name": task.cp,
                "version": task.version,
                "from": old,
                "action": action,
                "binary": task.type_name == "binary",
                "slot": task.slot,
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


def main(argv):
    if len(argv) < 2:
        say({"ok": False, "reason": "usage: bunny-portage.py plan [atom ...] | depclean"})
        return 2
    command, rest = argv[1], argv[2:]
    try:
        if command == "version":
            result = {"ok": True, "version": HELPER_VERSION}
        elif command == "plan":
            result = plan(rest)
        elif command == "depclean":
            result = depclean()
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
