# Cutting a candela release

Pushing a tag is the whole of it. Everything after that runs on its own and
leaves you one pull request to merge:

```sh
git tag v0.0.5 && git push origin v0.0.5
```

Tagging stays a decision, so nothing here creates a tag for you.

## Before you tag

Check `main` is green in the `ci` workflow.

Check the version in `Cargo.toml` is the one you are about to tag. It usually
is already, because the previous release opened a pull request that set it. If
it is not, run `scripts/bump-version.py <version>`, commit, push, and wait for
green. The tag has to match: `release.yml` compares the two before it builds
anything and `publish.yml` compares them again before it uploads, because the
MSI's version, the install receipt, and `candela --version` all come from the
manifest while the asset names come from the tag.

## What the tag sets off

`release.yml` checks the tag against the manifest, then calls
`build-artifacts.yml`, which builds the toolchain for each platform and
packages the archives, the Windows installer, the embedding library and the
WebAssembly build, and writes `sha256sums.txt` over the lot. `release.yml`
publishes them as the GitHub release. A leg that fails takes the release with
it, so a partial set of archives never goes out; fix what failed and re-run the
workflow.

Publishing the release starts two more workflows, both of which check out the
tag rather than `main`:

- `publish.yml` uploads `candela-vm` and then `candela-lang` to crates.io. A
  version already there is skipped, so re-running it is safe.
- `publish-extensions.yml` ships the VS Code extension to the marketplace and
  Open VSX, and the plugin to the JetBrains marketplace. Each step skips with a
  notice when its token is absent.

Then `release.yml`'s last job opens the pull request that moves `main` on.

## Verify

Install from the published release on a machine that has no candela on it, run
a program, and check `candela --version` reports the version you tagged.

## Merge the version bump

The pull request is titled `chore: set the version to X.Y.Z+1`, comes from a
branch named `bump-version-X.Y.Z+1`, and moves every place the tree writes its
version down. Merging it means a build from `main` calls itself the version the
next release will be tagged as, which is what leaves the step above with nothing
to do next time.

A number with no tag behind it is inert. Nothing turns a version into a download
address: every version-keyed lookup asks the releases page what exists, so a
version that has not shipped resolves to nothing and says so.

Its checks do not start on their own, because Actions opened it. Either approve
them in the merge box or close and reopen the pull request.

The bump is always the next patch, whatever kind of release the tag was. To go
somewhere else, run `scripts/bump-version.py 0.2.0` on that branch and push.

If no pull request appears, look at the `open the version bump` job in the
release run. It reports what it decided, and does nothing when the decision is
not its to make:

- The job never ran, because the release job did not finish. Fix what failed
  and re-run the workflow.
- `is not a plain vX.Y.Z tag`. Prereleases and other tag shapes are left alone,
  because what follows one is a choice. Run `scripts/bump-version.py` yourself.
- `main is at N, at or past ...`. The bump already landed, or this is a re-run
  of an older release after a later one went out. Nothing to do.
- `the tree would not come out at N`. A version literal changed shape, or one
  appeared somewhere new. The job names the file and the number it found; teach
  `bump-version.py` about it and bump by hand this once.

Re-running a release is safe here too. The job derives its branch name from the
version it is setting and commits the same content, so it lands on the branch it
used the first time and adds nothing to a pull request that is already open.

## The nightly channel

`nightly.yml` calls the same `build-artifacts.yml` every night and publishes
what comes out as a prerelease on one rolling tag, `nightly`, force-moved to
the commit it built. Nothing about it is yours to run, but two things about it
are worth knowing when you are about to tag.

A red nightly is the first news that the release build is broken, and it is
news you get on a quiet day too, because it runs whether or not `main` moved.
Read it the same way as `ci`: green before you tag.

It cannot cut a release. The tag is `nightly`, so it misses the `v*` trigger
that starts `release.yml`, and the release is a prerelease, so `publish.yml`
and `publish-extensions.yml` stop on their own and every version-keyed lookup
skips it. It publishes no Windows installer: the package stamps the version
from the manifest and Windows compares packages by that number, so a nightly
installer would be indistinguishable from a released one carrying the same
number.

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
