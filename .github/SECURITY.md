# Security policy

## Supported versions

Candela has no tagged release yet. The latest `main` is the only supported
version; fixes land there.

## Reporting a vulnerability

Report privately through GitHub security advisories: open the Security tab of
this repository and use "Report a vulnerability". That opens a private thread
with the maintainers.

Do not open a public issue for a bug that is exploitable. Memory unsafety in the
VM, a `.cdlb` artifact that reads or writes outside what its source allows, and
anything that lets a script reach past the host embedding API all belong in a
private report.

Include the version, the platform, and a repro if you have one. You get an
acknowledgement within a few days, and an update when the fix is ready or the
report is closed.
