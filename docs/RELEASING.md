# Releasing bunny

A release is a version tag (`v0.1.0`) on `main`. Pushing the tag makes GitHub
build bunny and publish a release with the binary attached. Nothing else is
manual.

## When

When a few features have piled up **and** have run on a real machine for a
while, not on every merge. Versions follow semantic versioning: while bunny is
0.x, bump the minor version (`0.1.0` → `0.2.0`) for new features or changed
behaviour, and the patch version (`0.2.0` → `0.2.1`) for fixes only.

## Steps

1. On a branch `release/v0.2.0`:
   - Set `version = "0.2.0"` in the top-level `Cargo.toml` (under
     `[workspace.package]`), then run `cargo build` so `Cargo.lock` updates.
   - In `CHANGELOG.md`, rename `## [Unreleased]` to `## [0.2.0] - <date>`
     (dates as `2026-11-30`) and add a new empty `## [Unreleased]` above it.
2. Open a PR titled "Release v0.2.0" and merge it once the checks pass.
3. Tag `main` and push the tag. The commands are for the maintainer's machine:

   ```sh
   git checkout main && git pull
   git tag -a v0.2.0 -m "bunny 0.2.0"
   git push origin v0.2.0
   ```

4. Watch **Actions → Release**. When it's done, the release is on the
   repository's **Releases** page, with:
   - `bunny-x86_64-linux`: one file that runs on any x86-64 Linux
     (statically linked, so it doesn't depend on the system's libraries)
   - `bunny-x86_64-linux.sha256`: its checksum
   - the release notes, taken from that version's section of
     `CHANGELOG.md`
   - a few minutes later, from **Actions → ISO**: the live USB image
     `bunny-linux-live-<version>.iso` and its `.sha256`. That workflow builds
     the image with `tools/build-iso.sh` and boots it in QEMU first; if it
     doesn't start, nothing is attached.

If the tag doesn't match the version in `Cargo.toml`, or `CHANGELOG.md` has no
section for it, the release stops before publishing anything and says which.

## Installing a release

```sh
curl -LO https://github.com/bunnyos-dev/bunnyos/releases/latest/download/bunny-x86_64-linux
curl -LO https://github.com/bunnyos-dev/bunnyos/releases/latest/download/bunny-x86_64-linux.sha256
sha256sum -c bunny-x86_64-linux.sha256
sudo install -m 755 bunny-x86_64-linux /usr/local/bin/bunny
```

(Later, the `bunny-overlay` ebuild installs releases with `emerge`.)

## Trying the live USB image without releasing

The ISO workflow also runs on pull requests that change the image
(`tools/build-iso.sh`, `tools/iso/`), and from **Actions → ISO → Run
workflow**. The ISO is kept for 7 days as a download on the run's page
(`bunny-linux-live-preview`). To build it yourself:

```sh
doas tools/build-iso.sh target/x86_64-unknown-linux-musl/release/bunny
tools/bunny-vm.sh run bunny-linux-live.iso
```

## Trying the release build without releasing

The release workflow also runs on pull requests that change it, and from
**Actions → Release → Run workflow**: it builds the binary and the notes and
keeps them as a workflow artifact, without publishing anything.
