# Disposable integration databases

From my2pg, run:

```sh
rtk run './tests/run-integration.sh mysql84 pg16 --smoke'
rtk run './tests/run-integration.sh mysql84 pg16'
```

`--smoke` verifies database startup and 12 independent seed assertions. The default also automatically discovers all ignored Rust integration tests with `bin/cargo test --locked --tests -- --ignored --nocapture`; missing tests/databases or zero executed passing cases fail the command. Every run gets a random Compose project and `target/integration/<project>/` artifacts, including exact image pins, actual versions/architecture, seed hashes, SQL assertions and logs. Only loopback ports are published. Cleanup verifies ownership labels on every container in that project (including orphans) and removes that project's containers/network/anonymous volumes; it never enumerates resources for global deletion.

For a worker needing live databases:

```sh
rtk run './tests/run-integration.sh mysql84 pg16 --start'
# Source the printed path's sibling env.sh, then run the requested test target.
rtk run './tests/run-integration.sh --stop /absolute/path/to/connections.json'
```

Connection env exports include `MY2PG_MYSQL_URL`, `MY2PG_MYSQL_ROOT_URL`, `MY2PG_POSTGRES_URL`, `MY2PG_TLS_CA`, `MY2PG_TLS_BAD_CA`, `MY2PG_TLS_CLIENT`, `MY2PG_TLS_CLIENT_KEY`, `MY2PG_INTEGRATION=1`, and artifact directory. Passwords are synthetic integration-only credentials. TLS is enabled for both servers, with a fresh two-day CA/server certificate valid for localhost/127.0.0.1. MySQL's `my2pg` account uses default caching_sha2_password; `tls_client` requires an X509 client certificate and has source SELECT grants. Wrong-host and unrelated CA certificate fixtures are also generated. Tests must independently prove TLS verification and authentication; seed readiness alone does not prove a Rust TLS connection.

`images.json` is the reviewed pin registry. A legacy single-platform entry keeps its existing fields, including `platform`; a multi-platform lane uses `{"platforms":{"linux/arm64":{...},"linux/amd64":{...}}}`, where each nested record has its own immutable digest and a `platform` value exactly matching its key. The harness selects only the native Linux architecture and records lane keys such as `mysql57@linux/amd64`, both selected digests, and the platform in its run metadata. It never uses emulation to satisfy a lane. Unknown lanes, malformed variants, missing native pins, and incompatible source/target platform pairs fail explicitly.

The ignored-case inventory in `tests/support/harness.py` has one explicit `run`, `not_applicable`, or `pending` disposition for each MySQL version. A `not_applicable` entry names the concrete source feature/version reason and is reported as excluded, not passed. Any `pending` case blocks a full lane before containers start. The corrected MySQL 5.7 inventory has 133 statically reviewed run candidates and 23 source-feature incompatibilities; this does not prove a runtime pass. The French PostgreSQL error case belongs to a separate NLS-enabled test target and is validated independently. Native AMD64 CI lanes cover MySQL 5.7 against PostgreSQL 16/17/18, while the existing MySQL 8 lanes use ARM64 pins. Never reuse Strangler databases, weaken MySQL authentication or count an unrun matrix lane as passed.

## Audited pgloader baselines

Build the read-only upstream reference into separate images from the workspace parent:

```sh
rtk run 'docker build --target pgloader-v3-builder --build-arg DYNSIZE=4096 -f pgloader/clojure/tests/Dockerfile -t my2pg-reference-v3:231ab867 pgloader'
rtk run 'docker build -f pgloader/clojure/Dockerfile -t my2pg-reference-v4:231ab867 pgloader/clojure'
rtk run 'python3 my2pg/tests/support/baseline.py v3 /absolute/owned/connections.json'
rtk run 'python3 my2pg/tests/support/baseline.py v4 /absolute/owned/connections.json'
```

These helpers only address an owned project, require the audited source revision and capture loader image IDs, commands, load hashes, logs and independent SQL checks. The 2026-10-01 evidence is preserved under `../fixtures/baselines/2026-10-01/`: v3 passed six checks on three rows; v4 returned success but failed exact BLOB content. Initial v3 PG URI/trust failures are retained, followed by the successful retry using its accepted URI and OpenSSL CA environment. MySQL8.4 authentication was not weakened. Baseline elapsed time is informational; this tiny fixture, dynamic build-dependency resolutions and concurrent development do not establish the release speed/RSS targets.
