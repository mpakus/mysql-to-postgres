# my2pg

A Rust command-line application in development for migrating **Oracle MySQL data and database structures to PostgreSQL**.

Build locally with `./bin/buld` (macOS or Linux), or use `./bin/buld linux` to build Linux through Docker on macOS. Archives and checksums are written to `my2pg/target/dist/`. See [build instructions](my2pg/docs/releases.md) for CPU targets and prerequisites. Every branch push runs the root release workflow and publishes an incrementally versioned development build for macOS/Linux on Intel and ARM to [GitHub Releases](https://github.com/mpakus/mysql-to-postgres/releases) after all build checks pass.

The Rust application lives in `my2pg/`. It provides offline `check`, read-only `inspect`/`plan`, bounded streaming `run`, saved-run `verify`, and a strict one-way `.load` importer. Read [configuration and CLI](my2pg/docs/config-and-cli.md) and [scope and compatibility](my2pg/docs/scope-and-compatibility.md) before using a real dataset. Development builds are available; the full compatibility, recovery, performance and release acceptance gates remain separate.

Start from the [example TOML](my2pg/docs/examples/mysql-to-postgres.toml), supply its credential references, and use the extracted binary:

```sh
my2pg check migration.toml
my2pg plan migration.toml
my2pg run migration.toml
my2pg verify migration.toml --run-dir runs/<completed-run> --mode content
```

Use `run` only after reviewing the plan and preparing source retention/recovery. Content verification requires its supported type subset and a consistent source snapshot or an unchanged frozen source; counts/schema alone do not prove data equality.

Migration safeguards include exact unsigned/decimal values, original bytes for explicit `bytea` casts, preserved AUTO_INCREMENT floors, required index/constraint finalization, bounded memory and explicit partial/indeterminate failure reports. TINYINT(1) stays integer unless explicitly mapped to boolean.

The [Strangler branch comparison](my2pg/docs/strangler-pgloader.md) adds an opt-in `zero-dates-to-epoch` transform that preserves genuine NULLs and avoids a separate sentinel-restoration UPDATE pass for fresh migrations. Content checks now include dates and microsecond timestamps, and unsupported requested content coverage fails before copying. Application-specific rounding, exclusions, Rails migration ledgers and CDC/cutover remain explicit operator work; no throughput improvement is claimed without a benchmark.

See [development and disposable tests](my2pg/docs/development.md), [architecture](my2pg/docs/architecture.md) and the [documentation index](my2pg/docs/README.md). Reference checkouts are source material, not runtime dependencies.

## Container

Build the Linux container from the repository root with `docker build -t my2pg:local my2pg`. The image uses the pinned Rust toolchain and runs as UID 10001. It contains the application, CA certificates, OpenSSL runtime and license notices. Pass operator configuration and credentials at runtime:

```sh
mkdir -p runs
docker run --rm --user "$(id -u):$(id -g)" --env-file .env \
  --volume "$PWD/migration.toml:/migration.toml:ro" \
  --volume "$PWD/source-ca.pem:/source-ca.pem:ro" \
  --volume "$PWD/target-ca.pem:/target-ca.pem:ro" \
  --volume "$PWD/runs:/runs:rw" \
  my2pg:local run /migration.toml
```

Set the source and target `ca_file` paths in `migration.toml` to `/source-ca.pem` and `/target-ca.pem`, and set `report.directory` to `/runs`. Keep `.env` private; it supplies `MY2PG_MYSQL_URL` and `MY2PG_POSTGRES_URL`. The `--user` option lets the report directory remain owned by the invoking host user.

Run the disposable container acceptance with `rtk proxy ./my2pg/tests/run-container-smoke.sh`. It builds the current source image and verifies a real TLS migration against the pinned MySQL/PostgreSQL fixture.

my2pg is distributed under the [PostgreSQL License](my2pg/LICENSE), as declared in `my2pg/Cargo.toml`. Reviewed upstream test fixtures retain their attribution and license terms under `my2pg/tests/fixtures/upstream/`.
