---
description: Break work into small verifiable tasks with acceptance criteria and dependency ordering
---

Load the `planning-and-task-breakdown` skill with the `skill` tool.

Read the existing spec (SPEC.md or equivalent) and the relevant codebase sections. Then:

1. Enter plan mode — read only, no code changes
2. Identify the dependency graph between components
3. Slice work vertically (one complete path per task, not horizontal layers)
4. Write tasks with acceptance criteria and verification steps
5. Add checkpoints between phases
6. Present the plan for human review

Save the plan to tasks/plan.md, the task list to tasks/todo.md, and per-module task chunks to tasks/<module-id>.md (one per module in the capability map). Each chunk opens with links to its SPEC-<module-id>.md and the files it touches, followed by its tasks with acceptance, verify, and files. Chunk file = the unit /build consumes; plan/todo are aggregates of the chunks.

If tasks/plan.md or tasks/todo.md already exists with unchecked tasks for different work, stop and ask before writing — never silently overwrite an incomplete plan.
