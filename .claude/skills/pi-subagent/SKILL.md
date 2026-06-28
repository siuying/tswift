---
name: pi-subagent
description: Spawn a pi subagent to handle a specific task in a separate process. Use when the user asks to delegate work to a subagent, run a task in parallel, or says "spawn a subagent", "use a subagent", or "run this with pi".
---

# pi-subagent

Spawn a separate `pi` process as a subagent, wait for it to finish, and read its output.

## Pattern

```bash
# 1. Write the prompt to a file
cat > /tmp/my-prompt.md << 'EOF'
<your task here>

When finished, write exactly `<<<DONE>>>` on its own line at the very end.
EOF

# 2. If unsure what model/provider to use, list available options first
pi --list-models

# 3. Spawn the subagent (cd into the project root so tools resolve correctly)
cd /path/to/project
pi --print \
  --provider "anthropic" \
  --model "claude-opus-4-5" \
  --name "my-subagent" \
  "$(cat /tmp/my-prompt.md)" \
  > /tmp/my-output.md 2>&1 &

SUBAGENT_PID=$!

# 4. Poll for the termination token
for i in $(seq 1 80); do
  sleep 15
  if grep -q '<<<DONE>>>' /tmp/my-output.md 2>/dev/null; then
    echo "✅ Done after $((i*15))s"
    break
  fi
  echo "⏳ $((i*15))s — $(wc -l < /tmp/my-output.md) lines so far…"
done

# 5. Read the output
cat /tmp/my-output.md
```

## Key flags

| Flag                        | Purpose                                           |
| --------------------------- | ------------------------------------------------- |
| `--print`                   | Non-interactive: process prompt and exit          |
| `--provider "<provider>"`   | Provider to use (e.g. `anthropic`, `openai-codex`) |
| `--model "<model>"`         | Model to use                                      |
| `--name "…"`                | Names the subagent session                        |

## Notes

- **No `--cwd`** — `pi` has no `--cwd` flag. Use `cd /path` before the command.
- **Keep tools enabled** (don't pass `--no-tools`) so the subagent can read files and run `git`/`bash`.
- **Termination token** — always ask the subagent to write `<<<DONE>>>` as its last line so you can poll without guessing when it's finished.
- **Prompt via file** — for long prompts, write to a file and use `"$(cat /tmp/prompt.md)"` to pass it; avoids shell quoting issues.
- The subagent inherits the current working directory, so `cd` into the project root first so relative paths in tools resolve correctly.

## Provider / model options

```bash
# Anthropic (Claude)
pi --provider "anthropic" --model "claude-opus-4-5" …
pi --provider "anthropic" --model "claude-sonnet-4-5" …

# OpenAI Codex (GPT)
pi --provider "openai-codex" --model "gpt-5.5" …

# Quick sanity check
echo "say hi" | pi --provider "anthropic" --model "claude-sonnet-4-5" --no-session --print
```

## When to use

- Delegating a long-running or self-contained task (e.g. implementing a feature, running a research pass) so it doesn't block the main session.
- Running multiple tasks in parallel by spawning several subagents and polling all of them.
- Isolating work that might produce noisy output or consume many tool calls.
