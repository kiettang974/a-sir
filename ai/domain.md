# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`ai/INDEX.md`**: the map of the whole source. Start here to find the right file.
- **`ai/GLOSSARY.md`**: the domain terms.
- **`ai/adr/`**: read ADRs that touch the area you're about to work in.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved, and must create them at these paths, not at the repo root.

## File structure

Single-context. Everything for agents lives in `ai/`; only `AGENTS.md` stays at the root, because tools read it only there.

```
/
├── AGENTS.md            ← points into ai/
├── ai/
│   ├── INDEX.md         ← table of contents for the source
│   ├── GLOSSARY.md
│   ├── domain.md        ← this file
│   ├── issue-tracker.md
│   ├── triage-labels.md
│   └── adr/
│       └── 0001-<decision>.md
├── crates/asr-core/
├── app/
└── ui/
```

When a file or module is added, renamed or removed, update `ai/INDEX.md` in the same change.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `ai/GLOSSARY.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
