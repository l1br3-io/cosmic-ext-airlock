# Airlock — Agent Skills

This project uses the [addyosmani/agent-skills](https://github.com/addyosmani/agent-skills) pack, installed under `.agents/` and `.opencode/`. Reusable workflows (skills), shared checklists (references), lifecycle slash commands, and specialist personas are all wired up and auto-discovered.

## Locations

| Asset | Path |
|-------|------|
| Skills | `.agents/skills/<name>/SKILL.md` |
| Shared checklists | `.agents/references/*.md` |
| Slash commands | `.opencode/commands/*.md`, mirrored to `.zcode/commands/*.md` for ZCode |
| Personas (subagents) | `.opencode/agents/*.md` |

## Core rules

- If a task matches a skill, load it with the `skill` tool before acting.
- Follow the skill workflow strictly — do not partially apply it.
- Never skip required steps (spec, plan, test) a skill demands.
- Every change clears the project-wide Definition of Done in `.agents/references/definition-of-done.md`.
- Commands are silent on success and announce on failure: slash commands give a concise result on success and surface failures prominently instead of burying them.

## Intent → skill mapping

- New feature / significant change → `spec-driven-development`, then `incremental-implementation` + `test-driven-development`
- Planning / breakdown → `planning-and-task-breakdown`
- Bug / failure / unexpected behavior → `debugging-and-error-recovery`
- Code review → `code-review-and-quality`
- Refactoring / simplification → `code-simplification`
- API or module boundary design → `api-and-interface-design`
- Rust code / review → `rust-best-practices`
- Switcher overlay visual design (colors, spacing, glyphs, glance legibility), gesture-time presentation, or user-facing copy in the CLI, config diagnostics, or docs → `airlock-design`
- Security-sensitive input, auth, secrets, dependencies → `security-and-hardening`
- Performance requirements or suspected regressions → `performance-optimization`
- Making a change (always) → `git-workflow-and-versioning`

## Slash commands

`/spec`, `/plan`, `/build [auto]`, `/test`, `/review`, `/constraints`, `/code-simplify`, `/ship`, `/webperf`. Run `/ship` before merge for a parallel fan-out review. In ZCode the planning command is `/planning` — `/plan` is a reserved built-in there, so its mirror is renamed to `planning.md`; opencode keeps `/plan`.

## Personas

`code-reviewer`, `security-auditor`, `test-engineer`, `web-performance-auditor` — single-role subagents invoked by `/ship` / `/webperf` (or directly). **Personas do not invoke other personas**; orchestration belongs to slash commands and the main session (see `.agents/references/orchestration-patterns.md`).

## Docs

- A shipped feature ships its user doc: a `docs/` page or section, linked from README's Documentation index.
- A feature with a research record gets (or updates) its row in `docs/research/README.md` — status lives in the index; the records themselves are never edited after the fact.
- Committed docs never reference the gitignored `SPEC*.md`/`tasks/` artifacts — checklist rows and prose must be self-contained.

## Rust project notes

- Run `cargo clippy` and `cargo fmt --check` before a change counts as done; `cargo test` is the verification gate.
- Unit tests live inline in `#[cfg(test)] mod tests`. When a test module outgrows ~500 lines, move it to a sibling `tests.rs` (e.g. `src/app/tests.rs`) declared as `#[cfg(test)] mod tests;` — it stays a child module, keeping private access and cfg gating while keeping whole-file reads cheap for agents. Never move unit tests to a top-level `tests/` dir; that is integration-test territory and loses private access.
- This is a COSMIC Wayland app: runtime behavior is only testable on real COSMIC hardware; unit tests cover what can be tested headless.
- **Never run the app on the local host.** Airlock takes over Alt+Tab, binds Wayland input, and paints overlays — launching it here hijacks the developer's real desktop session. Any execution of the `airlock` binary (`cargo run`, `just run`, running `./target/**/airlock` or `~/.cargo/bin/airlock` directly) is host-forbidden; so is `just install` on the host, because the spawn stub re-executes the daemon on the next key press. Runtime and manual testing happen only inside the COSMIC VM via the `just vm-*` recipes — see `docs/vm-development.md` — snapshotting first (`just vm-snapshot`) when exercising install/uninstall or crash paths. Headless commands (`cargo build`, `cargo test`, `just check`, `just fmt-check`) are safe locally.

## Spec & task artifacts (local working docs)

- `SPEC.md` is the master index: the capability map (module ids, dependency
  direction, build order) and per-module status. Per-module specs live at
  `SPEC-<module-id>.md`; per-module task chunks at `tasks/<module-id>.md`.
  Global aggregates: `tasks/plan.md` and `tasks/todo.md`.
- All of the above are local working artifacts, gitignored via `SPEC*.md`
  and `/tasks`. Never commit them.
- Module ids come from the capability map in `SPEC.md`, are chosen once, and
  are never renamed mid-initiative.
- Before implementing: read `SPEC.md` for orientation, then load the
  module's `SPEC-<id>.md` and `tasks/<id>.md` chunk. Never implement a
  module whose spec is not approved or whose chunk is not written.
- `/build` consumes the chunk files; see `.opencode/commands/build.md` (mirrored at `.zcode/commands/build.md`).
- PRs must copy the module's success criteria into the PR body, since
  reviewers cannot see the local spec files.

## File creation

Default to fewer, larger, cohesive files. File boundaries should follow
what changes together, not how much there is.

Extract a new file or subdirectory only when **all** of these are true:

- **The trigger has fired** — during a single feature or task, you keep
  editing unrelated sections of the same file.
- **One concept, one name** — the new unit maps to a concept you can name
  in a word or short phrase (`stringset`, `auth` — never `util`,
  `common`, `misc`), and the name works as a meaningful prefix for
  everything inside it.
- **It stands on its own** — its contents change together and don't need
  what they're being extracted from.

Before extracting, try in-file structure first: grouping related items
together, section comments, or narrowing scope often resolves the
discomfort without a new file.

Length alone is never a reason to split. Size is a lagging indicator —
it confirms a boundary you already noticed, it never creates one. The
typical failure mode: a long file becomes `utils`, `helpers`, and
`types` — same content, names that describe nothing, and every future
edit spans three files. When in doubt, don't extract; a long file is
cheaper to live with than a wrong boundary.

The bias runs in reverse too: sibling files that always change together
are merge candidates.

This guidance assumes module/namespace-oriented languages (Rust, Go,
JavaScript/TypeScript). It does not apply to frameworks that enforce
one-class-per-file patterns.
