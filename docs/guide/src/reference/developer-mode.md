# Developer mode: sending reports

For people testing Bunny Linux. With developer mode on, bunny records every
command exactly as it looked in your terminal (questions, answers, progress,
warnings), and `bunny report` sends the recordings to the developers as a
GitHub issue. Nobody has to copy and paste output, and problems you didn't
notice still reach someone who'll look at them.

> Developer mode is off unless you turn it on. Nothing is recorded or sent
> otherwise.

## Turn it on

Add this to `/etc/bunny/bunny.toml`:

```toml
[dev]
report = "auto"     # record everything; send problems by themselves
name = "laptop"     # what to call this computer in reports
```

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `report` | `false` / `true` / `"auto"` | `false` | `true` records every command; you send them with `bunny report`. `"auto"` also sends a command by itself when it fails or shows a ⚠ or ✗. Notes marked `·` (like the reminder to only add repositories you trust) don't count, and neither do your own slips: a package name that doesn't exist, a config mistake, or no password at sudo's question (exit code 4; see [exit codes](commands.md#exit-codes)). Those are still recorded, and `bunny report` sends them if you ask. |
| `name` | any text | `"this computer"` | Shown in reports, so you can tell your computers apart. |
| `reports` | `"owner/name"` | `"bunnyos-dev/bunny-reports"` | The GitHub repository reports are filed in. It should be private. |

Sending uses the GitHub CLI with your own account, so install it and sign in
once:

```sh
bunny add dev-util/github-cli
gh auth login
```

## Send what's been recorded

```sh
bunny report
```

It says how many commands are waiting and asks before sending. Each
recording is sent once. To see exactly what would be sent, without sending
anything:

```sh
bunny report --dry-run
```

## What a report contains

- Each recorded command: what you typed, everything bunny showed, how it
  ended (worked, or the exit code) and how long it took (for an install,
  until its recording was copied into the new system). Commands with a
  problem are opened up; the rest are folded.
- `bunny status` at the moment of sending. Working that out can take a
  moment, mostly the first time after starting the computer (it reads
  every Gentoo security advisory), so `bunny report` says "Adding `bunny
  status` to the report…" while it waits. If it takes over a minute, it's
  left out of the report and the report is sent without it.
- bunny's version, the kernel version, the Gentoo profile and the Portage
  version.

## What's taken out first

Before anything leaves the computer, bunny swaps out:

| Found | Becomes |
|---|---|
| user names (from `/etc/passwd`, and the user running bunny) | `you` |
| home folders (`/home/<name>`) | `/home/you` |
| the computer's name | `computer` |
| email addresses | `<email>` |
| Tailscale machine names (`*.ts.net`) | `<tailnet-name>` |

It can't know every private thing a command might print (a file name you
typed, say), which is why the repository should be private, and why
`--dry-run` is there.

## Where recordings are kept

In `~/.local/state/bunny/sessions/`: a `.log` file with the raw recording
and a `.toml` file with the command, the time and whether it's been sent.
Delete them whenever you like.

## Recording an install (preview USBs)

The live USB has no `bunny.toml`, so there's nowhere to turn developer mode
on. Instead, **preview USBs** (the ones built for testing, named
`bunny-linux-live-preview.iso`) record every bunny command by themselves:
they have a file, `/etc/bunny-preview.toml`, with `[dev] report = true`. The
welcome says so. Nothing is sent from the USB.

When `bunny install` finishes, it copies its recording (up to just before it
unmounts the new system, so the last few lines are missing) into your new
home folder, `~/.local/state/bunny/sessions/`. This works the same from
the terminal and from the graphical installer. After restarting, sign in to
GitHub and run `bunny report`: the install is one of the recordings waiting,
and it shows what it sends and asks first. It's sent like any other
recording, with personal details swapped out. If the install fails, the
recording stays on the USB, in `/root/.local/state/bunny/sessions/`, until
you restart.

Release USBs don't have the file and don't record anything. An install from
any USB is also recorded when developer mode is already on for that command
(for example, a `bunny install` you run inside a recorded session).

## How it works

bunny runs the command inside `script` (from util-linux), which records a
terminal exactly as it appears without changing how the command behaves.
Only commands typed in a terminal are recorded; `bunny report` itself and
commands run from scripts aren't. When bunny re-runs itself through sudo or
doas, the same recording continues.

One exception: `bunny install` run without a terminal (the graphical
installer runs it with its output going to the installer, not a terminal)
is recorded too. `script` needs a terminal, so bunny runs the install again
with its output passing through bunny, which hands every byte on unchanged
and keeps a copy. The password the installer sends on standard input goes
straight to the install and is never in the recording. `bunny install
--list-disks`, which the installer asks for often, isn't recorded.
