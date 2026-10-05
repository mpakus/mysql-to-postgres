# T22 modern Oracle MySQL fixture version expectations

## Research before editing

Narrow exclusive lease: tests/driver_spike.rs, tests/cases/t05_mysql.rs and this log. No production, registration, image, harness, board or index changes; no database lifecycle. Applied RTK/Ponytail (available/read at the documented local paths); choose two direct finite assertions, no new compatibility helper or skip policy.

XERJ health green/100% active shards, no pending tasks. Current coverage report checked2026-10-02T14:59:59.578903Z:320files/1173passages at db28c459a8a3caf45b07e9e72bcbc9fd3194c762; positively includes both leased tests, src/mysql/mod.rs, scope-and-compatibility.md and images.json. Current HEAD same; dirty working source authoritative. No coordinator index rebuild performed.

Fresh helper queries: project-my2pg `server_version starts_with Oracle MySQL 8.0 8.4` returns driver_spike:1 and source validator:121/301; ref-mysql_async `server_version mysql version 8.0 8.4` returns azure-pipelines:61 and README:421/src/lib:421 at immutable d7525dcb1d35f3d60101a2e95e84171c3d3ac4a5. Read original driver1–180, t05_mysql1–150/220–275, source validator95–130, scope7–9, pinned README418–447 and adjacent upstream Azure8.0/8.4 TLS matrix previously read. Upstream supports testing8.0 but does not certify our value/catalog semantics; do not adopt its mutable tags or global setup.

Intended modern fixture contract: scope explicitly plans Oracle5.7/8.0/8.4; these two cases use modern8.x behaviors (including8.0 invisible/CHECK/descending-index metadata), so keep their finite scope **8.0 or8.4**, not arbitrary MySQL/MariaDB versions or5.7. Production validate_server separately requires recognized Oracle Community/Enterprise and rejects known variants. Existing driver vendor assertion remains; t05 inspection retains production validation. Auth fixture continues caching_sha2_password and TLS/mTLS/CA/hostname assertions unchanged. Every raw unsigned/decimal/zero-date/time/blob/COPY and ordered catalog/default/PK/FK/check/readonly assertion remains unchanged.

Positive native provenance: existing reviewed Oracle mysql:8.0.46 linux/arm64 child digest213bbfaf699693a40a20a12bb4342d2589a15a3dc7153db698eaed252a92458e independently hashed3047bytes; image configMYSQL_VERSION8.0.46-1.el9/source7cf11d5360282effadb347353d5f82339506b106. Prior owned native run ceb5e6f1e341 independently reports8.0.46/PG16.15,14seed checks pass; both failures occur solely at8.4-only assertions (driver40–42/t05:72). The native case behavior beyond those assertions remains unproved. Historical failure is preserved in T22-mysql80-lane.md.

Chosen adaptation: each assertion accepts exact8.0. or8.4. version-line prefix and gives a finite modern fixture error message. Do not rename/relabel/skip cases, widen production supported versions or certify compatibility. Offline scoped rustfmt check, driver target compile/no-run and strict all-target/all-feature Clippy; coordinator owns subsequent native rerun.

## Implementation and offline verification

Changed only the two version-line assertions and their diagnostic messages. Vendor/TLS/auth/raw-value/catalog assertions, ignored fixture prerequisites and exact test IDs remain unchanged. No extra helper, table/seed mutation or version skip added.

- Scoped Rust1.99 rustfmt `--edition 2024 --check tests/driver_spike.rs tests/cases/t05_mysql.rs`: exit0.
- `rtk proxy bin/cargo test --offline --locked --features artifact-worker-tests,native-import-tests --test driver_spike --no-run`: exit0, driver target compiled0.75s; no test/database executed.
- `rtk proxy bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings`: exit0,1.24s.
- `rtk proxy git diff --check`: exit0; inspected diff is exactly the two expectations.

Frozen test SHA256: driver_spike.rs `896e8fc87609230137c67800e28e086b7878118e9f0cd26a8255100d86c09c95`; t05_mysql.rs `e8804fa3779a7e874f6dbb6712f687a83d3e97fe9b955d596183d231d5950846`. No native fixture started or stopped; actual8.0 behavior past the corrected gates awaits the coordinator's full requested rerun. This change removes a contradictory fixture prerequisite, not a compatibility acceptance.

## Coordinator full-lane acceptance

The complete owned serial command `rtk run python3 tests/support/harness.py mysql80 pg16` passed after the fixture expectations were narrowed. It ran **146 invoked tests, zero failures**, with all 15 required Darwin native IDs passing exactly once, against the pinned Oracle MySQL **8.0.46** ARM64 child digest and PostgreSQL **16.15**. This proves the requested 8.0→16 lane for the current host; it does not certify 8.0→17/18, other host architectures, or the remaining T22 matrix gates. The first failure artifact `my2pg-mysql80-pg16-ceb5e6f1e341` and the intermediate T11-oracle failure `my2pg-mysql80-pg16-c4746670c93b` remain retained. The accepted rerun is `target/integration/my2pg-mysql80-pg16-ac14bec5040c`: `rust-tests.json` SHA256 `346406f98ae3ea7f76fcec31f69810b3325da457e641f0a638e98f0376e9c50f`, `rust-tests.log` SHA256 `38e241bdeb57feb606c06bff38abf1c4f96cbf48a673e2ab0ec0441259edb603`; fixture metadata records `state=stopped`.

## Superseding lane evidence after T11/T17 registration

The later coordinator run `target/integration/my2pg-mysql80-pg16-737703545a93/` supersedes the prior 146-case result: it passed **147 invoked tests, zero failures**, with **16 required Darwin IDs** (including T11 truncate/reset, T17 ON UPDATE omission, and T17 generated-column policy) each exactly once. The fixture metadata records `state=stopped`; its native log SHA256 is `85d8cf1bbac1fd2e9d6a0c0a176239a0ecc25d7a3642a52903923c13f44a1967`. See the generated-column coordinator evidence for source revision, harness hash, and details. Earlier failed artifacts remain intact.
