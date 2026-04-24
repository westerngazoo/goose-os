## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
Don’t assume. Don’t hide confusion. Surface tradeoffs.
If the model is uncertain, it should ask. If multiple interpretations exist, it should name them instead of picking silently. If a simpler approach exists, it should say so. This principle is a direct response to the silent-assumption problem Karpathy described.

Minimum code that solves the problem. Nothing speculative.
No features beyond what was asked. No abstractions built for single-use code. No flexibility that wasn’t requested. The internal test: would a senior engineer call this overcomplicated? If yes, simplify.

Touch only what you must. Clean up only your own mess.
Don’t improve adjacent code, don’t reformat things that weren’t part of the task, don’t delete comments the model doesn’t understand. Every changed line should trace directly to what was actually requested.

Define success criteria. Loop until verified.
Instead of telling the model what to do step by step, describe what done looks like. LLMs are better at iterating toward a goal than following a rigid procedure. Give it a test to pass. Let it loop.
