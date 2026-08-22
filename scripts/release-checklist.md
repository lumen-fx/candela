# Cutting a candela release

Pushing a tag builds the toolchain, publishes the release, sends the crates and
the editor extensions out after it, and leaves `main` already carrying the next
version:

```sh
git tag v0.0.5 && git push origin v0.0.5
```

Tagging stays a decision, so nothing here creates a tag for you.

## Before you tag

Check `main` is green in the `ci` workflow.

Check the version in `Cargo.toml` is the one you are about to tag. It usually
is already, because the previous release set it. If it is not, run
`scripts/bump-version.py <version>`, commit, push, and wait for green. The tag
has to match: `release.yml` compares the two before it builds anything and
`publish.yml` compares them again before it uploads, because the MSI's version,
the install receipt, and `candela --version` all come from the manifest while
the asset names come from the tag.

## What the tag sets off

`release.yml` checks the tag against the manifest, then calls
`build-artifacts.yml`, which builds the toolchain for each platform and
packages the archives, the Windows installer, the embedding library and the
WebAssembly build, and writes `sha256sums.txt` over the lot. `release.yml`
publishes them as the GitHub release. A leg that fails takes the release with
it, so a partial set of archives never goes out; fix what failed and re-run the
workflow.

It then calls `publish.yml` and `publish-extensions.yml`, and commits
`chore: set the version to X.Y.Z+1` straight to `main`, so the tag is the last
thing a release asks anyone to type. Only `main` moves; the release keeps the
tree the tag pointed at.

## The crates and the extensions

`publish.yml` uploads `candela-vm` and then `candela-lang` to crates.io.
`publish-extensions.yml` ships the VS Code extension to the marketplace and Open
VSX, and the plugin to the JetBrains marketplace. Each step skips with a notice
when its token is absent.

`release.yml` calls both in the same run rather than leaving them to start on
the release they follow. The release is created with the workflow token and a
workflow token raises no events, so a workflow waiting on a published release
waits forever. Being called also puts both on the release run's ref, which is
the tag, so each reads the version that was released rather than the one `main`
moves to.

Re-run a leg by hand when it fails, and give it the tag rather than `main`. The
tag carries the version that was released; `main` has already moved past it, and
a run off anything but a `vX.Y.Z` tag verifies and uploads nothing.

```sh
gh workflow run publish.yml --ref v0.0.5 -f dry_run=false
gh workflow run publish-extensions.yml --ref v0.0.5
```

A version already on crates.io is skipped, so running `publish.yml` again is
safe. `publish-extensions.yml` has no such check; it takes one run at a time per
tag, so nothing races, but re-run it for a leg that has not uploaded yet rather
than for one that has.

## Verify

Install from the published release on a machine that has no candela on it, run
a program, and check `candela --version` reports the version you tagged. Check
the crate versions on crates.io and the extension listings while you are there.

## The version bump

`release.yml`'s `move main to the next version` job runs
`scripts/bump-version.py`, which moves every place the tree writes its version
down, and pushes the commit to `main` over SSH with the `BUMP_DEPLOY_KEY`
deploy key. The key is what gets past the `main` ruleset, which names it as a
bypass actor, and a push made with it raises events, so the bump commit gets a
`ci` run like any other commit on `main`.

A build from `main` now calls itself the version the next release will be tagged
as. That number is inert until the tag exists: nothing turns a version into a
download address, and every version-keyed lookup asks the releases page what
exists, so a version that has not shipped resolves to nothing and says so.

The bump is always the next patch, whatever kind of release the tag was. To go
somewhere else, run `scripts/bump-version.py 0.2.0` on `main` afterwards.

The job reports what it decided. It stops without committing when the decision
is not its to make:

- `is not a plain vX.Y.Z tag`. Prereleases and other tag shapes are left alone,
  because what follows one is a choice. Run `scripts/bump-version.py` yourself.
- `main is at N, at or past ...`. The bump already landed, or this is a re-run
  of an older release after a later one went out. Nothing to do.

It fails, rather than reporting and stopping, when the bump is owed and it
cannot make it. `main` would otherwise go on calling itself a version that is
already released, and the next tag of that number gets rejected for disagreeing
with the manifest:

- `BUMP_DEPLOY_KEY is not set`. Add the secret and re-run the workflow.
- `pushing N was refused`. The deploy key is no longer a bypass actor on the
  `main` ruleset, or `main` is moving faster than the job can follow. It takes
  `main` again and re-decides twice before giving up.
- `the tree would not come out at N`. A version literal changed shape, or one
  appeared somewhere new. The job names the file and the number it found; teach
  `bump-version.py` about it and bump by hand this once.

Re-running a release is safe here too. The job reads the version off `main` each
time round rather than off the tree the run started with, so a second run finds
`main` already at the number it would set and commits nothing. The push is never
forced, so a `main` that moved during a long build refuses the push instead of
losing what moved it.

## The nightly channel

`nightly.yml` calls the same `build-artifacts.yml` every night and publishes
what comes out as a prerelease on one rolling tag, `nightly`, force-moved to
the commit it built. Nothing about it is yours to run, but two things about it
are worth knowing when you are about to tag.

A red nightly is the first news that the release build is broken, and it is
news you get on a quiet day too, because it runs whether or not `main` moved.
Read it the same way as `ci`: green before you tag.

It cannot cut a release, and it cannot reach crates.io or either marketplace.
The tag is `nightly`, so it misses the `v*` trigger that starts `release.yml`,
and `release.yml` is the only thing that calls the two publish workflows;
nothing else can start them but a hand dispatch. Both refuse to upload off a ref
that is not a `vX.Y.Z` tag, so `nightly` gets no further even when dispatched at
it. What the nightly publishes is a prerelease, which every version-keyed lookup
skips.

It publishes no Windows installer: the package stamps the version from the
manifest and Windows compares packages by that number, so a nightly installer
would be indistinguishable from a released one carrying the same number.

## What the bump script moves

`scripts/bump-version.py` takes the version to set and finds the places that
carry it by what they are rather than from a list: the `candela-lang` and
`candela-vm` package versions, the `candela-vm` pin in the root manifest, and
both crates' entries in `Cargo.lock`. It writes nothing at all unless every one
of them comes out at the version you asked for, so a literal it was never taught
about stops the bump instead of shipping a version skew.

Two kinds of version stay where they are. The language server and the editor
extensions are numbered as the tools they are, on their own schedule, and the
script leaves them alone by name rather than by what number they happen to
carry. So does every version that names a release that exists, such as the
dependency line in `docs/docs/integration/embedding.md`, which tells an embedder
what to ask crates.io for. The version the script sets is what the tree will be
next, which is a different thing.
