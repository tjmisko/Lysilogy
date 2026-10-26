#!/bin/bash
# Fable via the Claude Code CLI on the plan (claude.ai auth). One call per goal.
P=$HOME/.cache/lysilogy-scratch/e2-prompts
OUT=$HOME/Projects/Lysilogy/.worktrees/feat/reading-lists/eval/claims/e2/raw/fable
for g in "$@"; do
  env -u ANTHROPIC_API_KEY -u OPENROUTER_API_KEY -u OPENAI_API_KEY claude --print --model claude-fable-5-1 --tools "" --output-format json --system-prompt "$(cat $P/instructions.md)" --json-schema "$(cat $P/schema.fable.json)" < $P/$g.input.md >| $OUT/$g.json
  echo "$g exit $?"
done
