# Repository Guidance

This file defines repository-wide guidance. More specific instructions belong
in nested `AGENTS.md` or `AGENTS.override.md` files close to the code they
govern.

## Required Project Documentation

Every repository using this guidance maintains these canonical documents:

- `README.md`: the human-facing entry point and source of truth for setup,
  development, build, test, lint, type-check, release, and operations commands.
- `docs/agent-onboarding.md`: the agent-facing project orientation and
  documentation index.
- `docs/codebase-map.md`: the routing index from packages, concerns, and key
  symbols to the source files that own them.

Create or bring all three documents into compliance when adopting this
guidance. After adoption, keep them current whenever a change affects facts
they own. Do not duplicate the same fact across documents; link to its canonical
home instead.

## Starting Work

1. Read `docs/agent-onboarding.md` for the project purpose, stack,
   infrastructure, invariants, conventions, and documentation routes.
2. Read `docs/codebase-map.md` before planning code changes, then confirm its
   routing against the source.
3. Read the relevant README sections for exact commands and operational
   workflows.
4. Read every more-specific `AGENTS.md` or `AGENTS.override.md` that applies to
   the files being changed.

The source and checked-in configuration remain authoritative. Fix stale routing
or documentation when the current task changes or depends on it. Report
unrelated documentation gaps without broadening the task.

## Accuracy, Recency, and Sources

- Use Context7 for targeted library or API documentation. Identify the library
  and target version, fetch only the relevant material, and summarize it rather
  than copying large excerpts.
- Use web search when current external information materially affects
  correctness, including APIs, release notes, compatibility, and security
  advisories.
- Prefer official documentation and primary sources. Use reputable secondary
  sources only when primary material is insufficient.
- Record publication or release dates when recency affects a decision.

## Agent Onboarding

`docs/agent-onboarding.md` is the concise agent entry point, not a second README
or an operations manual. It owns:

- a one- or two-sentence description of the project;
- a brief view of the stack and infrastructure topology;
- repository conventions and invariants stated as rules;
- a documentation index that routes each question to its canonical source.

It links to exact versions and configuration in manifests, file and symbol
routing in `docs/codebase-map.md`, and human development or operations
instructions in `README.md`. Keep out copied commands, configuration values,
file inventories, algorithm descriptions, and per-file walkthroughs.

## Codebase Map

`docs/codebase-map.md` is a concise routing aid, not an implementation guide or
exhaustive inventory. It contains:

- a one- or two-sentence repository overview;
- a compact, package-grouped list of routing-critical files and entry points,
  with their responsibility and useful search symbols;
- a short "By concern" section mapping cross-cutting behavior to its owners.

Update the map when files, entry points, or responsibilities change. Keep out
algorithm descriptions, control-flow prose, directory trees, dependency
inventories, commands, workflows, and contribution guidance. Prefer curated
routes over one entry for every file or public symbol.

## Verification and Completion

Use the commands documented in `README.md`. Run focused checks first, then the
test, build, lint, format, and type-check commands prescribed there. Work is
complete when the requested behavior is verified, affected canonical documents
are current, and the diff contains no unrelated changes. Report commands run,
results, and checks that could not be completed.
