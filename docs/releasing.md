# Releasing Airlock

A release is three public artifacts telling one story: an annotated
`v<version>` git tag, a GitHub Release carrying that version's changelog
section, and the crate on crates.io. The whole sequence is five `just`
recipes, and the four that touch the public world are run by the owner
by hand — never by an agent, never by CI. Typing the command is the
consent; the tokens that make it work (`gh auth`, `cargo login`) live
on the owner's machine and nowhere in this repository.

## What has to agree before you start

One version, in four places, checked mechanically by the gate:

| Where | What |
|---|---|
| `Cargo.toml` | `version = "X.Y.Z"` |
| `CHANGELOG.md` | the top section header: `## [X.Y.Z] — YYYY-MM-DD` |
| `res/io.github.l1br3-io.Airlock.metainfo.xml` | `<release version="X.Y.Z" date="YYYY-MM-DD">` — same date |
| the tag | `vX.Y.Z` at the release commit (created by the runbook) |

The changelog entry is written for users, newest first, phrased around
what they can observe — it *is* the release notes, published verbatim.
Date it the day you publish. README's status line carries the same
version and date.

## Before you release

Nothing public moves until all of these are true:

1. **The changelog entry is complete** — every user-visible change on
   `main` has a line; nothing internal-mechanical. Read it as the user
   would; it is the text that appears on the release page.
2. **README's install instructions are true** — or become true the
   moment you publish (the `cargo install` path), with no stale version
   numbers or dates.
3. **The VM walk passed.** From a snapshot, `just vm-push` →
   `just vm-install` → the gesture, icon, and desktop entry all work →
   `just vm-uninstall` leaves the guest as it was. See
   [Developing and testing in the COSMIC VM](vm-development.md). This
   is the only honest end-to-end test — never run the binary on the
   desktop you are typing in.
4. **Review happened.** Changes since the last release were reviewed;
   anything found was fixed or explicitly waived.
5. **`just release-check` is green** at the exact commit you will tag,
   with no commits after it. If anything lands in between, run it
   again — the gate proves *this* tree, not an earlier one.

## The runbook

Run in order from the release commit:

```bash
just release-check            # 1. the gate
just release-notes 0.1.0      # 2. notes out of the changelog
just release-tag 0.1.0        # 3. tag + push
just release-gh 0.1.0         # 4. GitHub Release
just release-publish          # 5. crates.io
```

What each does, and what you will see:

**`release-check`** runs the quality gates (`just test`, `check`,
`fmt-check`), proves the one-version story above (versions equal, the
two dates equal), validates the metainfo with `appstreamcli` (one
documented warning is allowed — the app ID's hyphen, which matches the
GitHub org), lists the crate's files, and rehearses the publish with
`cargo publish --dry-run --locked`. Silent when green; names the
offending pair when not. Needs `appstreamcli` on the host (Debian
package `appstream`).

**`release-notes <ver>`** copies that version's changelog section —
without its header — to `target/release-notes-<ver>.md`. Mechanical on
purpose: the words you approved are the words that get published.

**`release-tag <ver>`** refuses a version `Cargo.toml` does not carry,
then creates the annotated tag `v<ver>` at `HEAD` and pushes it. If the
tag already exists it is *force-moved* and says so — see "Re-cutting"
below for when that is legitimate.

**`release-gh <ver>`** points the GitHub Release at the notes file:
edits the existing release object in place when there is one (the URL
survives), creates it otherwise, marks it latest. Needs `gh` logged
in.

**`release-publish`** is `cargo publish --locked` — the crate built
exactly as the committed `Cargo.lock` pins it. This is the one step you
cannot take back: crates.io versions are immutable. Needs `cargo login`
done once.

## After publishing

- The crates.io page for the version is live.
- The README install command works verbatim. Verify without touching
  your desktop's install: build into a throwaway home —
  `CARGO_HOME=$(mktemp -d) cargo install cosmic-ext-airlock --version X.Y.Z`
  — and check the binary landed. Do not run it on the host; the VM walk
  already proved it runs.
- Re-read README once more: the install promise is now true.

## Re-cutting a version

A tag may be moved **only while that version has never reached
crates.io**. Published versions are immutable; a fix to a published
version is a new version. Before the first publish, though, re-tagging
is the cleaner choice over shipping an `X.Y.1` nobody could have
installed — the tag moves, the release page follows the tag, and the
changelog entry is re-dated. `release-tag` performs the move and
announces it; `release-gh` keeps the release URL.

## Rules of the road

- **Owner-run, every public step.** Tag, release page, publish. An
  agent may run `release-check` (it only reads and rehearses) and may
  prepare the changelog, metainfo, and README — it never pushes or
  publishes.
- **No commits between the gate and the tag.** The gate proves the
  tree it ran on.
- **One version story.** The four places above never disagree; the
  gate refuses to let them.
- **Secrets stay out.** `gh` and `cargo` credentials are the owner's;
  nothing in the repository carries them.
- **Publish last.** Tag and release page are re-runnable; the crate is
  not. Verify everything you can before step 5.

## Quick reference

| Recipe | Touches the public world? | Needs |
|---|---|---|
| `release-check` | no (dry-run only) | `appstreamcli` |
| `release-notes <ver>` | no | — |
| `release-tag <ver>` | yes — pushes the tag | push access |
| `release-gh <ver>` | yes — release page | `gh auth login` |
| `release-publish` | yes — crates.io, irreversible | `cargo login` |
