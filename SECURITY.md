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
  `$XDG_RUNTIME_DIR` or, failing that, a `noiseman-<uid>` directory in the temp dir, which must be owned by the user
  with mode 0700; the socket is 0600. On Windows it lives in `%LOCALAPPDATA%\noiseman`, whose ACL admits only the user,
  SYSTEM and administrators.
- Anything running as the same user can control the player and read its log; that is by design, not a vulnerability.
- The installers download over HTTPS from GitHub Releases and check each archive against its published SHA-256 before
  installing. They trust GitHub and the HTTPS connection; the binaries are not code-signed.
