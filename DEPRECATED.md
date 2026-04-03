# Deprecated Components

These components are from the v0.3.0 "Agentic OS" era and are being phased out per the 2026-04-03 product pivot to Memory Infrastructure + Guardrails.

## Skills (will be removed)
- forge:forge -- agent team orchestration router
- forge:forge-feature -- code modification workflow
- forge:forge-new -- greenfield project workflow
- forge:forge-review -- code review orchestration
- forge:forge-ship -- PR/merge workflow
- forge:forge-handoff -- session handoff

## Skills (keeping, may be updated)
- forge:forge-setup -- daemon prerequisite checks (keep, update for v0.4.0)
- forge:forge-agents -- agent status viewer (keep, update)
- forge:forge-security -- security scanning (keep, integrate with guardrails)
- forge:forge-research -- research loop (keep)

## Agents (will be removed)
- forge-planner -- replaced by daemon's LSP-based code intelligence
- forge-generator -- agent orchestration dropped
- forge-evaluator -- agent orchestration dropped

## What replaces them
The Forge daemon (crates/daemon/) provides memory + guardrails as infrastructure.
Agent orchestration is delegated to the host agent (Claude Code, Gemini, Codex, etc.) and their native skill systems (Superpowers, etc.).
