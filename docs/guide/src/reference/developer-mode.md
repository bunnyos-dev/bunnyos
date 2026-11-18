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
| `report` | `false` / `true` / `"auto"` | `false` | `true` records every command; you send them with `bunny report`. `"auto"` also sends a command by itself when it fails or shows a ⚠ or ✗. |
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
  ended (worked, or the exit code) and how long it took. Commands with a
  problem are opened up; the rest are folded.
- `bunny status` at the moment of sending.
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

## How it works

bunny runs the command inside `script` (from util-linux), which records a
terminal exactly as it appears without changing how the command behaves.
Only commands typed in a terminal are recorded; `bunny report` itself and
commands run from scripts aren't. When bunny re-runs itself through sudo or
doas, the same recording continues.
