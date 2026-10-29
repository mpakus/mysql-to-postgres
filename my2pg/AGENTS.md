# my2pg instructions

- Always use `rtk` and `ponytail` skills/guidance when writing or improving code.
- Before coding, follow [reference coding](docs/reference-coding.md): search XERJ for the problem, read matching source and tests, and record evidence and adaptation in the task worklog.
- Keep the [MySQL-only scope](docs/scope-and-compatibility.md), [task ownership](docs/agent-tasks.md), and [completion checklists](docs/checklists.md). The coordinator owns reference refreshes and index rebuilds.
- For casts, defaults or verification, consult [Strangler migration lessons](docs/strangler-pgloader.md). Check source reads, row/default conversion, transformation counts and verification together. Preserve NULL separately from zero-date sentinels; validate requested verification coverage before target writes. Test exact keyed values and same-count corruption, not counts alone.
