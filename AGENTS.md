# AGENTS.md

Read all `.ai/rules/*.mdc` files for coding conventions, stack decisions,
and process rules. Cursor users: rules are also in `.cursor/rules/`.

---

## Research & Library Usage

**Check the current date before researching.** Your training data may be stale.
When using a library, search for latest docs first. Verify you're using
the current API, not a deprecated one.

---

## Critical Rules

### Always
- Type-annotate all function signatures
- Validate at system boundaries (user input, external APIs, CLI args)
- Use structured logging (`structlog`/`pino`) — never `print()` or `console.log()`
- Run `just check` before claiming work is complete

### Never
- Commit secrets, `.env` files, or credentials
- Add a dependency for something achievable in <20 lines
- Skip tests when adding new logic or fixing bugs
- Use `Any` or untyped interfaces without explicit justification

### Ask First
- Adding new dependencies or changing the stack
- Schema changes or data migrations
- Changing auth flows, permissions, or security boundaries
- Architectural decisions that affect multiple components

---

## Project Context

<!-- Fill in below. For deeper domain knowledge, create docs/DOMAIN.md -->

### Overview
<!-- What does this project do? Who is it for? -->

### Goals
- [ ] Goal 1

### Non-Goals
- Not building X

### Technical Constraints
- Deployment target: [platform]

### Domain Context
<!-- Key terms, business rules, entities.
     If this section grows beyond a few bullets, move it to docs/DOMAIN.md
     and reference it here. See the DOMAIN.md guide below. -->

---

## Building Domain Knowledge

As you work on this project, you'll learn domain-specific context that
future agents (and your future self) will need. Capture it:

1. **Start here** — fill in the Project Context section above with basics
2. **Grow into `docs/DOMAIN.md`** — when domain context outgrows a few bullets,
   create a dedicated file covering:
   - **Glossary** — key terms and their precise meanings in this domain
   - **Entities & relationships** — the core data model in plain language
   - **Business rules** — constraints that aren't obvious from the code
   - **User journeys** — the 2-3 critical paths through the system
3. **Keep it alive** — update domain docs when you learn something new.
   Stale domain docs are worse than none.

This is project-owned — adapt the structure to what your domain actually needs.
