# Helping test: send what confused you

Bunny Linux is young, and the best way to make it better is to hear where
it's confusing. **Developer mode** records every bunny command exactly as you
saw it, and sends recordings with problems to the developers by itself, so
you don't have to copy and paste anything.

Set it up right after installing, before you start changing things.

## 1. Sign in to GitHub

Reports are sent as issues in a private repository, with your own GitHub
account, so sign in first. Otherwise the first problem bunny tries to send
only says it couldn't send it. Install GitHub's command-line tool and sign in
once:

```sh
bunny add dev-util/github-cli
gh auth login
```

Pick GitHub.com, HTTPS, and "Login with a web browser". Without a desktop,
`gh` says it can't copy the code and can't open a browser. That's expected:
it still shows a one-time code. Open `github.com/login/device` on your phone
or another computer, type the code, and `gh` finishes by itself.

## 2. Turn it on

```sh
sudo nano /etc/bunny/bunny.toml
```

Add at the end:

```toml
[dev]
report = "auto"     # record everything; send problems by themselves
name = "laptop"     # what to call this computer in reports
```

Save (Ctrl+O, Enter), close (Ctrl+X), and check:

```sh
bunny check
```

## 3. Use Bunny Linux

That's it. Every bunny command is recorded. One that fails, or shows a ⚠ or
✗, is sent by itself.

To send everything that's been recorded, including things that worked but
were confusing:

```sh
bunny report
```

It asks before sending, and `bunny report --dry-run` shows exactly what
would go. Your user name, host name, home folder and emails are replaced
with placeholders first. Every detail is in
[Developer mode, in detail](../reference/developer-mode.md).

## Also worth writing down

Recordings show what bunny printed, not what you expected. When something
surprised you, a line or two helps a lot: what you were trying to do, what
you expected, and which page of this Guide you were following. Add it as a
comment on the report's issue, or open an issue on
[github.com/bunnyos-dev/bunnyos](https://github.com/bunnyos-dev/bunnyos/issues).

## Turning it off

Remove the `[dev]` section, or set `report = false`. Nothing is recorded or
sent after that.
