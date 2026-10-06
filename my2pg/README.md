# my2pg

A Rust CLI for migrating Oracle MySQL structures and data to PostgreSQL. Read the [scope and compatibility contract](docs/scope-and-compatibility.md) and [configuration/operator guide](docs/config-and-cli.md) before running a migration. Compilation alone does not certify the server/platform matrix or complete the release acceptance gates.

From a source checkout, build a release archive with `./bin/build-macos` on macOS or `./bin/build-linux` on Linux. The Linux script also works through Docker on macOS. See [release builds](docs/releases.md) for CPU targets, Windows experiments, prerequisites and checksum verification.

Extracted binary archives contain `my2pg` (or experimental `my2pg.exe`), operator documentation, examples, license notices, build provenance and checksums. Run `my2pg --help` to inspect commands; `check` validates configuration offline and `plan` inspects connected catalogs before execution.
