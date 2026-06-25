---
name: subagent
description: Create subagents to handle specific tasks.
disable-model-invocation: true
---

If you are not Pi agent, skip this skill.

Use following commands to list provider/models:

Spawn a separate pi process as a subagent, wait for it to finish, and read its output.

## Pattern

```bash
# 1. Write the prompt to a file
cat > /tmp/my-prompt.md << 'EOF'
<your task here>

When finished, write exactly `<<<DONE>>>` on its own line at the very end.
EOF

# 2. Spawn the subagent (run from the project root so tools work)

# if unsure what model/provider to use, run `pi --list-models`
pi --list-models

# actually running the subagent
cd /path/to/project
pi --print \
  --provider "openai-codex" \
  --model "gpt-5.5" \
  --name "my-subagent" \
  "$(cat /tmp/my-prompt.md)" \
  > /tmp/my-output.md 2>&1 &

SUBAGENT_PID=$!

# 3. Poll for the termination token
for i in $(seq 1 80); do
  sleep 15
  if grep -q '<<<DONE>>>' /tmp/my-output.md 2>/dev/null; then
    echo "✅ Done after $((i*15))s"
    break
  fi
  echo "⏳ $((i*15))s — $(wc -l < /tmp/my-output.md) lines so far…"
done

# 4. Read the output
cat /tmp/my-output.md
```

## Key flags

| Flag                        | Purpose                                          |
| --------------------------- | ------------------------------------------------ |
| `--print`                   | Non-interactive: process prompt and exit         |
| `--provider "openai-codex"` | Use OpenAI Codex provider (required for gpt-5.5) |
| `--model "gpt-5.5"`         | Model to use                                     |
| `--name "…"`                | Names the subagent session                       |

## Notes

- **No `--cwd`** — `pi` has no `--cwd` flag. Use `cd /path` before the command.
- **Keep tools enabled** (don't pass `--no-tools`) so the subagent can read files and run `git`/`bash`.
- **Termination token** — always ask the subagent to write `<<<DONE>>>` (or similar) as its last line so you can poll without guessing when it's finished.
- **Prompt via file** — for long prompts, write to a file and use `"$(cat /tmp/prompt.md)"` to pass it; avoids shell quoting issues.
- The subagent inherits the current working directory, so `cd` into the project root first so relative paths in tools resolve correctly.

## Provider / model options

```bash
# GPT-5.5 (OpenAI Codex provider)
pi --provider "openai-codex" --model "gpt-5.5" …

# Anthropic (default when no provider specified)
pi --provider "anthropic" --model "claude-opus-4-5" …

# Quick sanity check
echo "say hi" | pi --provider "openai-codex" --model "gpt-5.5" --no-session --print
```
