# Security policy

## Supported versions

Candela is pre-1.0 and the current version is 0.0.3. Only the latest release is
supported: fixes land on `main` and ship in the next tagged release, and there
are no backports to earlier versions. Reproduce on the latest release before
reporting.

This covers the `candela` toolchain in this repository: the `candela` compiler
and runner, the `candela-vm` runtime, and the `candela-lsp` language server.

## What counts as a vulnerability

Candela is not a sandbox. A program can open any dynamic C library it names in a
`dylib` block, and a `.cdlb` artifact records those same import recipes, so
running an untrusted `.cdl` or `.cdlb` is equivalent to running an untrusted
native binary. Reports that rely on running a hostile program are out of scope.

In scope:

- Memory unsafety in `candela-vm`, including anything reachable from a
  malformed or hand-edited `.cdlb`.
- A crash, hang, or out-of-bounds access triggered by compiling untrusted
  source, since editors and the language server compile whatever a user opens.
- Anything that lets a script reach past the host embedding API into memory the
  embedder did not hand it.
- Anything in the install or update path that lets an attacker substitute a
  binary: a checksum that is not verified, or an artifact fetched over a channel
  that is not authenticated.

## Reporting a vulnerability

Report privately through GitHub security advisories: open the Security tab of
this repository and use "Report a vulnerability". That opens a private thread
with the maintainers. Do not open a public issue for a bug that is exploitable.

Include the version (`candela --version`), the platform, and a repro if you have
one. You get an acknowledgement within a few days, and an update when the fix
ships or the report is closed.
