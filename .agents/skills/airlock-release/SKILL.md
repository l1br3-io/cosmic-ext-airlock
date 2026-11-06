---
name: airlock-release
description: The owner-run release ritual for Airlock — walking a version through gate, notes, tag, GitHub Release, and crates.io publish with a verification checkpoint at every step, where the three public commands are always typed by the human, never the agent. Use whenever asked to release, publish, ship, or cut a version, when a tag or GitHub Release needs moving, or when verifying a publish after the fact.
---

<!-- Forged in the release-pipeline module's first exercise (2026-11-06);
     the committed maintainer guide is docs/releasing.md — this skill is
     the agent's execution discipline on top of it. -->

# Airlock Release

A release is three public artifacts telling one story — an annotated
`v<version>` tag, a GitHub Release carrying that version's changelog
section, and the crate on crates.io. Two of the five recipes that
produce them are safe for you to run; the three that touch the public
world are typed by the human, every time. Your job is the part around
their typing: gates before, presentation during, verification after.

## The contract

- **You never run `release-tag`, `release-gh`, or `release-publish`.**
  Not "rarely", not "when the user seems to want speed". Tag pushes,
  the release page, and the crates.io publish are the human's hands;
  their typing is the consent. If asked to just do it, restate this
  rule and present the command instead.
- **You may run `release-check` and `release-notes`** — the gate reads
  and rehearses, the extraction writes a local file — but announce
  each before running it.
- **Every step ends in verified evidence, never trust.** A step whose
  outcome you cannot observe (command exit, tag hash, release view,
  crates.io API) is not done.
- **`docs/releasing.md` is the reference.** If the skill and it ever
  disagree, the committed doc wins and the skill gets fixed.

## The step loop

Run every step the same way:

1. **GATE** — check the step's preconditions yourself, with commands.
2. **PRESENT** — show the human exactly what to run and what success
   looks like. For the public steps this is a hand-off, not a
   suggestion.
3. **WAIT** — stop. Do not run anything public. Do not pre-verify
   states that don't exist yet.
4. **VERIFY** — after they report back, check the observable evidence.
5. **REPORT** — one line: what passed, what's next. Then ask to
   proceed. No batching steps without an explicit go between them.

Any red at any point: **stop, report the evidence, propose the fix.**
Never push through a failed gate, and never "fix" a red gate by
re-running it until green without understanding why it was red.

## Preflight (before step 1)

- Which version, and confirm it everywhere it must agree:
  `Cargo.toml` `version`, the top `## [X.Y.Z] — date` header of
  `CHANGELOG.md`, the metainfo `<release version= date=>` — versions
  equal, the two dates equal, and the date is the publish day. The
  gate checks this mechanically; know it before you present anything.
- The changelog entry is the release notes. Say so; ask whether the
  human has read and approves it. No approval, no step 1.
- The VM walk for this tree passed (install/uninstall from snapshot —
  `docs/vm-development.md`). Ask; record the answer.
- Working tree clean, `main` pushed. A release cut from an unpushed or
  dirty tree is not a release.

## The steps

**1 — Gate.** Announce, then run `just release-check`. Record
`git rev-parse --short HEAD` alongside the green result — the gate
proves *that* commit. If any commit lands between now and the tag,
re-run the gate at the new HEAD; "it was green earlier" is not a gate.

**2 — Notes.** Announce, then run `just release-notes <ver>`. Verify
`target/release-notes-<ver>.md` is non-empty and matches the changelog
section (diff them). Show the human the file's first lines — these are
the words about to go public.

**3 — Tag (public, irreversible-ish).** Present:

```
just release-tag <ver>
```

Tell them what they'll see: the recipe refuses a version `Cargo.toml`
doesn't carry, and prints a **force-move notice** if the tag already
exists — expected and legitimate *only* when that version never
reached crates.io (a published version is immutable; fixing it means a
new version, never a moved tag). After their "done": verify
`git rev-parse v<ver>^{commit}` equals the gated HEAD and
`git ls-remote origin refs/tags/v<ver>` matches it too.

**4 — GitHub Release (public).** Present:

```
just release-gh <ver>
```

It edits the existing release object in place when there is one (the
URL survives) and creates it otherwise. After: `gh release view v<ver>`
— the notes are the step-2 file, the title is `Airlock <ver>`, and it
is marked latest. Anything else, stop and report.

**5 — Publish (public, irreversible).** The one step with no undo.
Before presenting it, say exactly that: crates.io versions are
immutable; this is the point of no return. Confirm the human wants it
now, then present:

```
just release-publish
```

(`cargo login` must have been done once on their machine; if the
command fails on credentials, that's their step, not yours.) After:
verify the version exists —

```
curl -s https://crates.io/api/v1/crates/cosmic-ext-airlock \
  | grep -o '"num":"<ver>"'
```

**6 — Post-publish verification.** The README's install promise is now
true; prove it without touching the desktop's own install:

```
CARGO_HOME=$(mktemp -d) cargo install cosmic-ext-airlock --version <ver>
```

Build-only — never run the binary on the host (AGENTS.md); the VM walk
already proved it runs. Then close the books per the house convention:
the module's status row and task boxes in the local working trackers
(SPEC*.md, tasks/ — gitignored, never committed), dates on everything.

## When it goes wrong

- Gate red → it names the offending pair; fix the disagreement, never
  the checker.
- Wrong commit tagged → re-run the gate at the right commit, then have
  the human re-run `release-tag` (force-move is the designed remedy,
  pre-publish only).
- Publish failed midway → nothing on crates.io is half-published; the
  failure output says why; fix and re-present step 5. If the version
  *did* land and something after it failed, the release is out —
  verify from the API and continue at the failed step.
- The human hesitates at any presentation → stop. A paused release is
  free; a wrong one is not.
