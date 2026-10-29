# my2pg

A Rust CLI for migrating Oracle MySQL structures and data to PostgreSQL. Read the [scope and compatibility contract](docs/scope-and-compatibility.md) and [configuration/operator guide](docs/config-and-cli.md) before running a migration. Compilation alone does not certify the server/platform matrix or complete the release acceptance gates.

The loader preserves exact decimal/integer values, raw bytes for explicit `bytea` casts and source sequence floors, with bounded COPY pipelines and explicit partial-failure reports. The [Strangler migration lessons](docs/strangler-pgloader.md) add opt-in full-zero-date conversion to an epoch sentinel while retaining genuine NULLs, plus exact date/microsecond timestamp content checks. Known unsupported content verification now fails planning before copying; a fresh sentinel migration needs no separate restoration UPDATE pass. Application-specific rounding, filtering and cutover choices stay explicit.

From a source checkout, build a release archive with `./bin/buld` on macOS or Linux. Use `./bin/buld linux` for Linux through Docker on macOS. The same entry point is available at the repository root. See [release builds](docs/releases.md) for CPU targets, automatic GitHub releases, prerequisites and checksum verification.

Extracted binary archives contain `my2pg` (or experimental `my2pg.exe`), operator documentation, examples, license notices, build provenance and checksums. Run `my2pg --help` to inspect commands; `check` validates configuration offline and `plan` inspects connected catalogs before execution.

Review `plan` before `run`; use `verify --run-dir <completed-run> --mode content` against unchanged source data for the supported type subset. See [configuration and CLI](docs/config-and-cli.md) for credential references, snapshots, transformation policies and recovery limits.
