# Security

## Reporting a vulnerability

Report it privately through [GitHub's vulnerability
reporting](https://github.com/roudy16/noiseman/security/advisories/new), not as a public issue. Expect a reply within a
week. Once a fix is released, the advisory is published with credit to the reporter, unless they would rather not be
named.

## Supported versions

Only the latest release gets fixes. The installers always fetch it.

## Threat model

- noiseman makes no network connections. The browser demo is a static page that loads nothing beyond its own files.
- The player and its clients talk over a local socket that only the same user can reach. On Linux and macOS it lives in
  `$XDG_RUNTIME_DIR` or, when that is unset or not private, a `noiseman-<uid>` directory in the temp dir, or, when that
  is not private (e.g. another user made it first), a `noiseman` directory in `$XDG_CACHE_HOME` or `~/.cache`, whose
  socket path must fit `sun_path`. Each must be an absolute path to a real directory (not a symlink) owned by the user
  with no group or other access (mode 0700); the daemon logs each one it passed over and why. The socket, lock file and
  log are 0600, and the lock file and log are opened without following symlinks and never truncated. On Windows it lives
  in `%LOCALAPPDATA%\noiseman`, whose ACL admits only the user, SYSTEM and administrators; `$XDG_RUNTIME_DIR`, which
  only the tests and gates set there, is taken as given.
- Anything running as the same user can control the player and read its log; that is by design, not a vulnerability.
- The installers download from GitHub Releases over HTTPS and check each archive against its published SHA-256 before
  installing. `install.sh` with curl refuses any other protocol or a TLS version below 1.2, redirects included, unless
  `NOISEMAN_BASE_URL` names another source. They trust GitHub and the HTTPS connection; the binaries are not
  code-signed.

## Verifying a release

Releases are immutable, and from the release after 0.1.2 every file in one (archives, `.sha256` files, `install.sh`,
`install.ps1`) has a build provenance attestation signed by the release workflow. To check a downloaded file with the
GitHub CLI:

```sh
gh attestation verify noiseman-x86_64-unknown-linux-gnu.tar.gz --repo roudy16/noiseman
```

It fails unless the file is byte for byte what `.github/workflows/release.yml` built from this repository.
