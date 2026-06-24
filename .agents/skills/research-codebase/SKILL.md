---
name: research-codebase
description: Conduct comprehensive research across the codebase to answer questions about how it works, where things live, and what patterns exist. Synthesizes findings from locating files, analyzing implementations, and extracting patterns — then saves a structured research document to docs/research/. Use when you need a thorough, documented answer about any aspect of the codebase.
---

# Research Codebase

You are tasked with conducting comprehensive codebase research to answer the user's question. You work directly — reading files, searching, and synthesizing — without delegating to sub-agents.

## CRITICAL: YOUR ONLY JOB IS TO DOCUMENT AND EXPLAIN THE CODEBASE AS IT EXISTS TODAY

- DO NOT suggest improvements or changes unless the user explicitly asks
- DO NOT perform root cause analysis unless the user explicitly asks
- DO NOT propose future enhancements unless the user explicitly asks
- DO NOT critique the implementation or identify problems
- DO NOT recommend refactoring, optimization, or architectural changes
- ONLY describe what exists, where it lives, how it works, and how components interact

## On Invocation

Respond with:

> I'm ready to research the codebase. Please provide your research question or area of interest, and I'll analyze it thoroughly by exploring relevant components and connections.

Then wait for the user's research query.

---

## Steps After Receiving the Research Query

### Step 1 — Read any directly mentioned files first

If the user mentions specific files, read them **fully** before doing anything else. This ensures you have full context before decomposing the research.

### Step 2 — Analyse and decompose the research question

- Break the query into composable research areas
- Identify specific components, patterns, or concepts to investigate
- Consider which directories, files, or architectural patterns are relevant
- Think carefully about underlying connections and implications

### Step 3 — Run research using the three codebase skills in sequence

Work through each relevant angle directly, in this order:

1. **Locate** — Apply the `codebase-locator` skill to find WHERE relevant files and components live. Search broadly: `Sources/`, `Tests/`, `Packages/`, `docs/`, configuration files.

2. **Analyse** — Apply the `codebase-analyzer` skill to understand HOW the most relevant components work. Trace data flow, entry points, and key logic with file:line references.

3. **Find patterns** — Apply the `codebase-pattern-finder` skill to surface existing patterns and concrete code examples that are relevant to the question.

Search thoroughly. Cover:
- Implementation files
- Test files
- Configuration and package manifests
- Documentation

### Step 4 — Gather metadata for the research document

Run these commands:

```bash
date -u +"%Y-%m-%dT%H:%M:%SZ"   # current timestamp
git rev-parse HEAD                # current commit hash
git branch --show-current         # current branch
git remote get-url origin         # repo URL
```

### Step 5 — Write the research document

Save to: `docs/research/YYYY-MM-DD-<description>.md`

- `YYYY-MM-DD` — today's date
- `<description>` — brief kebab-case description of the topic
- Example: `docs/research/2025-01-08-card-review-flow.md`

Structure:

```markdown
---
date: <ISO timestamp with timezone>
git_commit: <full commit hash>
branch: <branch name>
repository: <repo name>
topic: "<User's question/topic>"
tags: [research, codebase, <relevant component names>]
status: complete
last_updated: <YYYY-MM-DD>
---

# Research: <User's question/topic>

**Date**: <timestamp>
**Git Commit**: <commit hash>
**Branch**: <branch name>

## Research Question

<original user query>

## Summary

<High-level description of what was found, answering the user's question>

## Detailed Findings

### <Component / Area 1>

- What exists (`path/to/File.swift:line`)
- How it connects to other components
- Current implementation details

### <Component / Area 2>

...

## Code References

- `Sources/ModuleName/File.swift:123` — description
- `Tests/ModuleNameTests/FileTests.swift:45-67` — description

## Architecture Documentation

<Patterns, conventions, and design implementations found — no evaluation>

## Open Questions

<Any areas that need further investigation>
```

### Step 6 — Add GitHub permalinks (if applicable)

Check whether the current commit is pushed:

```bash
git branch --show-current
git status
gh repo view --json owner,name
```

If on `main` or the commit is pushed, replace local file references with GitHub permalinks:
`https://github.com/{owner}/{repo}/blob/{commit}/{file}#L{line}`

### Step 7 — Present findings

- Give a concise summary to the user
- Include key file references for easy navigation
- Ask if they have follow-up questions

### Step 8 — Handle follow-up questions

If the user follows up:
- Append to the same research document
- Update `last_updated` in frontmatter
- Add `last_updated_note: "Added follow-up for <description>"`
- Add a new section: `## Follow-up Research — <timestamp>`
- Continue researching and updating the document

---

## Important Notes

- Always run fresh research — never rely solely on existing research documents
- Focus on concrete file paths and line numbers for developer reference
- Document cross-component connections and how systems interact
- **You are a documentarian, not an evaluator** — describe what IS, not what SHOULD BE
- **No recommendations** — only document the current state
- Follow the numbered steps in order; never write the document with placeholder values
- Read mentioned files fully before starting research (Step 1 before Step 3)
- Gather metadata before writing the document (Step 4 before Step 5)
