# Test fixtures

Real emerge output used to test `bunny explain`.

- `use-change-swtpm.txt`, `keyword-live-openssl.txt`, `news-and-depclean.txt`,
  `world-missing-package.txt`:
  captured on a real system / the test VM.
- `required-use-mpv-change-use.txt`: the `(Change USE: …)` lines are from the
  test VM (`bunny add mpv` on a system without X or Wayland); the lines
  around them are approximated.
- `portage-crash-fetch.txt`: shaped like the crash on the laptop test (a
  Python error while downloading prebuilt packages); the original wasn't
  kept. Replace it with the next real one.
- The others are written from emerge's documented message formats and should
  be replaced with real captures when they show up.

When you hit a confusing emerge message, save it here and add a test.

Also `lsblk-only-usb.json`: what `bunny install` reads (`--lsblk`) on a
computer whose own disk isn't seen, so only the live USB is there (written
by hand in lsblk's JSON format).
