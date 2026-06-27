---
name: agent-loop
description: Process all pending issues for agent.
disable-model-invocation: true
---

Run the following command to get all issues labelled 'ready-for-agent' on github:

`gh issue list --state open --limit 100 --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`

For each single issue:

- create a feature branch `{username}/{issue-number}-{issue-title}`
- use `tdd` to implement them
- run `requesting-code-review` (with gpt-5.5) and fix any issues found by the code review
- create a PR from the branch, stacked on top of previous
  - Include a short summary of the changes in the PR description.
  - Include which issues this PR closes "Closes #<issue-number>".
  - For task with tests, include list of what is tested in the PR.
- If there are anything you cannot resolve, leave a comment to the issue and label it `ready-for-agent` and `blocked`.
- Do not merge the PRs, wait for human to review.

Work until all issues are implemented / blocked. Generate a clean and beautiful summary, in html of all implemented issues and their results, open it on browser.
