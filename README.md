# my2pg

A Rust command-line application in development for migrating **Oracle MySQL data and database structures to PostgreSQL**.

The Rust package builds and dispatches offline `check`, read-only `inspect`/`plan`, migration `run`, and a strict offline `.load` importer. These paths are being checked against disposable MySQL8.4/PostgreSQL16 databases; compatibility, recovery, concurrency and performance acceptance remain open. The importer supports a finite reviewed subset; see [configuration and CLI](docs/config-and-cli.md#strict-pgloader-importer) for flags and limits. This is not a release candidate. The sibling `pgloader/` checkout is the reference implementation.

Start with [configuration and CLI](docs/config-and-cli.md), [architecture](docs/architecture.md), and [scope and compatibility](docs/scope-and-compatibility.md). A runnable [MySQL-to-PostgreSQL example](docs/examples/mysql-to-postgres.toml) shows the supported connection and migration settings.

The current acceptance target is one source, one target, a bounded streaming COPY pipeline, inspectable migration plans, strict configuration, and useful terminal and JSON output. Performance comparisons must use correctness-checked pgloader runs.

## Container

Build the Linux container with `docker build -t my2pg:local .`. The image uses the pinned Rust toolchain to build the CLI and runs as UID 10001. It contains the application, CA certificates, OpenSSL runtime, project license, and third-party notices; it does not include database servers or Lisp/JVM tooling. Pass operator configuration and credentials at runtime:

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

Run the disposable container acceptance with `rtk run ./tests/run-container-smoke.sh`. It builds the current source image and verifies a real TLS migration against the pinned MySQL/PostgreSQL fixture.

my2pg is distributed under the [PostgreSQL License](LICENSE), as declared in `Cargo.toml`. Reviewed upstream test fixtures retain their own attribution and license terms under `tests/fixtures/upstream/`.
