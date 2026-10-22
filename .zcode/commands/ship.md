---
description: Run the pre-launch checklist via parallel fan-out to specialist subagents, then synthesize a go/no-go decision
---

Load the `shipping-and-launch` skill with the `skill` tool.

`/ship` is a **fan-out orchestrator**. It runs three specialist subagents in parallel against the current change, then merges their reports into a single go/no-go decision with a rollback plan. The subagents operate independently — no shared state, no ordering — which is what makes parallel execution safe and useful here.

## Phase A — Parallel fan-out

Spawn three subagents concurrently with the `subagent` tool. **Issue all three calls in a single turn so they execute in parallel** — sequential calls defeat the purpose of this command.

1. **`code-reviewer`** — Run a five-axis review (correctness, readability, architecture, security, performance) on the staged changes or recent commits. Output the standard review template.
2. **`security-auditor`** — Run a vulnerability and threat-model pass. Check OWASP Top 10, secrets handling, auth/authz, dependency CVEs. Output the standard audit report.
3. **`test-engineer`** — Analyze test coverage for the change. Identify gaps in happy path, edge cases, error paths, and concurrency scenarios. Output the standard coverage analysis.

These are defined in `.opencode/agents/` (fall back to agent `general` with an equivalent brief if one is missing). Each subagent gets its own context window and returns only its report to this main session — do not let one subagent delegate to another.

## Phase B — Merge in main context

Once all three reports are back, the main agent (not a subagent) synthesizes them:

1. **Code Quality** — Aggregate Critical/Important findings from the code review and any failing tests, lint, or build output. Resolve duplicates between reviewers.
2. **Security** — Promote any Critical/High security findings to launch blockers. Cross-reference with the review's security axis.
3. **Performance** — Pull from the review's performance axis; cross-check Core Web Vitals if applicable.
4. **Accessibility** — Verify keyboard nav, screen reader support, contrast (not covered by the three subagents — handle directly here, or consult `.agents/references/accessibility-checklist.md`).
5. **Infrastructure** — Env vars, migrations, monitoring, feature flags. Verify directly.
6. **Documentation** — README, ADRs, changelog. Verify directly.

## Phase C — Decision and rollback

Produce a single output:

```markdown
## Ship Decision: GO | NO-GO

### Blockers (must fix before ship)
- [Source: Critical finding + file:line]

### Recommended fixes (should fix before ship)
- [Source: Important finding + file:line]

### Acknowledged risks (shipping anyway)
- [Risk + mitigation]

### Rollback plan
- Trigger conditions: [what signals would prompt rollback]
- Rollback procedure: [exact steps]
- Recovery time objective: [target]

### Specialist reports (full)
- [code review report]
- [security audit report]
- [test coverage report]
```

## Rules

1. The three Phase A subagents run in parallel — never sequentially.
2. Subagents do not call each other. The main agent merges in Phase B.
3. The rollback plan is mandatory before any GO decision.
4. If any subagent returns a Critical finding, the default verdict is NO-GO unless the user explicitly accepts the risk.
5. **Skip the fan-out only if all of the following are true:** the change touches 2 files or fewer, the diff is under 50 lines, and it does not touch auth, payments, data access, or config/env. Otherwise, default to fan-out. `/ship` is designed for production-bound changes — when the blast radius is non-trivial, run the parallel review even if the diff looks small.
