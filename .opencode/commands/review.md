---
description: Conduct a five-axis code review — correctness, readability, architecture, security, performance
---

Load the `code-review-and-quality` skill with the `skill` tool.

Detect the tech stack first (this is Rust on libcosmic), then consult the `Intent → skill mapping` in AGENTS.md and load every skill the change's intent matches — at minimum `rust-best-practices` for the Rust code, plus `security-and-hardening` and `performance-optimization` when those axes are implicated.

Review the current changes (staged or recent commits) across all five axes:

1. **Correctness** — Does it match the spec? Edge cases handled? Tests adequate?
2. **Readability** — Clear names? Straightforward logic? Well-organized?
3. **Architecture** — Follows existing patterns? Clean boundaries? Right abstraction level?
4. **Security** — Input validated? Secrets safe? Auth checked? (`security-and-hardening`)
5. **Performance** — No N+1 queries? No unbounded ops? (`performance-optimization`)

Categorize findings as Critical, Important, or Suggestion.
Output a structured review with specific file:line references and fix recommendations.
