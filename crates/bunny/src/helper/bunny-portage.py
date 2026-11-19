"""bunny's Portage helper: asks Portage itself and answers in JSON.

bunny used to read emerge's screen text, which breaks whenever emerge changes
its wording. This script uses Portage's own library (the code emerge runs)
and prints exact data instead. It's shipped inside bunny and run with the
system's python3, which has Portage.

Usage (one JSON object on stdout; anything Portage prints goes to stderr):

    python3 bunny-portage.py plan [atom ...]

    plan: what `emerge --update --deep --newuse <atoms>` would install
          (default atom: @world), with the same options emerge would use
          (EMERGE_DEFAULT_OPTS and FEATURES=getbinpkg included).
          {"ok": true, "merges": [{"name", "version", "from", "action",
                                    "binary", "slot", "why"}]}
          "why" (rebuilds only): {"use": ["+flag", "-flag"], "libraries":
          ["cat/pkg"]}: the USE flags that changed, and the libraries it
          uses whose sub-slot changes in this plan (emerge's `r`).
          {"ok": false, "reason": "..."} when Portage can't work out a plan;
          bunny then asks emerge itself, to explain why.

Licence: GPL-3.0 (part of bunny).
"""

import json
import os
import shlex
import sys

HELPER_VERSION = 1


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
        return {"ok": False, "reason": "portage couldn't work out a plan"}

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


def main(argv):
    if len(argv) < 2:
        say({"ok": False, "reason": "usage: bunny-portage.py plan [atom ...]"})
        return 2
    command, rest = argv[1], argv[2:]
    try:
        if command == "version":
            result = {"ok": True, "version": HELPER_VERSION}
        elif command == "plan":
            result = plan(rest)
        else:
            result = {"ok": False, "reason": "unknown command: %s" % command}
    except Exception as e:  # never crash with a traceback on stdout
        result = {"ok": False, "reason": "%s: %s" % (type(e).__name__, e)}
    result["helper"] = HELPER_VERSION
    say(result)
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    os.environ.setdefault("PORTAGE_QUIET", "1")
    sys.exit(main(sys.argv))
