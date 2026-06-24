---
name: create-pr
description: Create a pull request in the warp repository for the current branch. Use when the user mentions opening a PR, creating a pull request, submitting changes for review, or preparing code for merge.
---

# create-pr

## Pre-PR Checklist

### 1. Merge master into your feature branch

**Always merge master into your feature branch before starting the review process.**

```bash
git fetch origin
git merge origin/master
```

Resolve any merge conflicts locally before opening the PR.

### 2. Run presubmit checks for code changes

If the PR includes code changes, run the relevant presubmit checks before opening or updating it:

```bash
./scripts/presubmit
```

`./scripts/presubmit` runs:

- `cargo fmt` - Code formatting
- `cargo clippy` - Linting with all warnings as errors
- All tests (unit, doc, and integration)
  If the PR is documentation-only (for example, skills, markdown, or other non-code content), you do not need to run `cargo fmt` or `cargo clippy` just to open or update the PR.

If presubmit fails for a code-changing PR, use the `fix-errors` skill to resolve issues.

**You must run `cargo fmt` and `cargo clippy` before:**

- Opening a new PR that includes code changes
- Pushing new commits that include code changes to an existing PR branch
- Any reviewed branch update that changes code

### 3. Review your changes

Before creating a PR, review what changes you're about to submit:

```bash
# View commits in your branch (comparing against base branch)
git --no-pager log <base-branch>..HEAD --oneline

# View file statistics for changes
git --no-pager diff <base-branch>...HEAD --stat

# View full diff
git --no-pager diff <base-branch>...HEAD
```

This helps you:

- Verify all intended changes are included
- Catch unintended changes before review
- Write an accurate PR description
- Ensure you're comparing against the correct base branch
- **Tests:** Include tests when required—bug fixes (regression test), algorithmic code (unit tests), UI components (layout test), P0 use cases (integration test). See Testing Requirements below.

### 4. Link to Github Issue

When possible, PRs should be associated with a Github issue. Use the Github CLI to find corresponding issues.

**Branch naming convention:**
Remote branches should be prefixed with your name (e.g., `zheng/feature`, `alice/fix-bug`).

**How to link PRs to Github Issues:**
Include the issue ID in the PR title. Do this **before** creating the PR for automatic linking.

### 5. Open the PR

Add changelog entries when appropriate using the format at the bottom of the PR template. Some examples:

- Feature: "Global search in files across your current directories. Use CMD-F/CTRL-SHIFT-F to open."
- Improvement: "Added horizontal autoscrolling when jumping to line/column."
- Bug fix: "Fixed session viewer input being cleared when agent runs commands.

**CLI workflow:**

- **Check if PR exists** for current branch:

  ```bash
  gh pr view --json number,url
  ```

  Exit code 0 if PR exists, 1 if not.

- **Create a new PR:**

  ```bash
  # With title and body
  gh pr create --title "Title" --body "Description" --draft

  # Auto-fill from commits
  gh pr create --fill --draft

  # Use PR template file
  gh pr create --body-file .github/pull_request_template.md --title "Title" --draft
  ```

  Key flags: `--draft` / `-d`, `--fill` / `-f`, `--body-file` / `-F`, `--web` / `-w`

- **Update an existing PR:**

  ```bash
  gh pr edit --title "New title" --body "New body"
  gh pr edit --add-reviewer username --add-label bug
  ```

- **Mark PR ready for review:**
  ```bash
  gh pr ready
  ```

## Testing Requirements

### Bug fixes require regression tests

**All bug fixes should be accompanied by a regression test.** This helps prevent re-breaking something that was already broken once.

The test should:

- Reproduce the original bug (would fail before the fix)
- Pass after the fix is applied
- Be clearly named to indicate what bug it's preventing

### Algorithmic code requires unit tests

Code with non-trivial logic should have unit tests to validate functionality:

**Examples of what needs unit tests:**

- Custom data structures (e.g., `SumTree`)
- Search-related APIs that should return expected results for a given query
- Core layout code in the UI framework
- Any algorithmic or computational logic

**Not required for:**

- Sufficiently-simple functions
- Trivial getters/setters

Follow the repository's local testing conventions for guidance on writing unit tests.

### Ask before skipping integration coverage

If the PR changes a user-visible flow, fixes an end-to-end regression, or otherwise looks like it would benefit from integration coverage, use the `ask_user_question` tool before creating or updating the PR to ask whether the user wants an integration test added as part of the work.

Prefer a direct choice such as:

- `Yes, add an integration test before creating the PR`
- `No, continue without an integration test`

If the user chooses to add one, use the `warp-integration-test` skill.

### P0 use cases require integration tests

**All "P0 use cases" require an integration test** that covers the behavior/flow in question.

**A "P0 use case" is defined as:** Any behavior of the application that, if broken, warrants an out-of-band release.

Integration tests should:

- Exercise the full user-facing flow
- Validate end-to-end functionality
- Be placed in the `integration/` directory

Use the `warp-integration-test` skill for implementation details, test registration steps, and validation workflow.

## PR Description Guidelines

Your PR summary under the "Description" section should include:

1. **What** - What changes are being made
2. **Why** - Why these changes are necessary (link to Linear task if applicable)
3. **How** - Brief explanation of the approach taken

## Best Practices

- **Keep PRs focused** - One logical change per PR when possible
- **Write clear commit messages** - Explain what and why, not just what
- **Self-review first** - Review your own diff before requesting review
- **Update tests** - Ensure test coverage reflects your changes
- **Document breaking changes** - Call out any API changes or breaking modifications
- **Use feature flags** - Gate risky changes behind feature flags when appropriate (see the `add-feature-flag` skill)
