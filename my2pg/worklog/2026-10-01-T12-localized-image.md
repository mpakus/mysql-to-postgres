# T12 native localized PostgreSQL fixture

Status: image pin preparation; real localized recovery evidence remains pending.

## Reference research before changing image pins

XERJ `project-my2pg / images.json harness isolation` at project revision `776901a1377fbea654bdf19f7a201014af757fbc` returned `tests/compose/images.json` and `tests/support/test_harness.py`. Read `harness.py:135-194`, isolation tests and the Compose PostgreSQL entrypoint: lane keys are data driven, reject unreviewed images/emulation, retain ownership labels and scoped cleanup. Reuse this existing harness; no new orchestration abstraction.

XERJ `ref-rust-postgres / sqlstate localized` at pin `1084ca8f5b5302e161892f2fa40abf71b4060c10` returned `tokio-postgres/src/error/mod.rs:181`. Read adjacent accessors: `code()` is SQLSTATE while `message()` and severity may be translated. Adaptation: prove recovery by the structured code with an actual non-English builtin error, never a trigger pretending to be a builtin error.

The current Alpine PG16 fixture lacks NLS. Official registry `docker buildx imagetools inspect postgres:16-bookworm --raw` resolved native linux/arm64/v8 image manifest `sha256:1c2f3efc9c5ab63fe557565c9443dbcfc0cecd2b28a3989244dfc80eb6cb96f9` with version `16.15-bookworm`, source revision `9d15534160ade17f2b6c455a39ee967c49b1937d`, and official docker-library provenance. Read its [original Dockerfile](https://github.com/docker-library/postgres/blob/9d15534160ade17f2b6c455a39ee967c49b1937d/16/bookworm/Dockerfile): installs locales and the PGDG PostgreSQL package on arm64; only en_US is generated initially. Record actual pg_config NLS support, installed message catalog and generated locale in the owned container before counting a localized test.

Chosen adaptation: add a separate `pg16-nls` exact manifest pin; preserve the existing pg16 development pin. Generate the available non-English locale only inside the unique disposable container if required. No host locale change, production connection or matrix acceptance claim. Worker owns this fixture lifecycle and its locale/recovery test evidence.

Before test registration, read Cargo.toml and harness.py:242-257: the main harness discovers all test binaries with --tests, so simply adding an ignored NLS case would run it on an image that lacks NLS and fail for the wrong lane. Use a separate Cargo test target with required `nls-integration` feature; main development harness still discovers all ordinary database modules, while the explicitly invoked NLS target requires its actual locale environment and fails if it is absent. No successful runtime skip. Native command: `bin/cargo test --locked --features nls-integration --test nls -- --ignored --nocapture`. Ordinary all-feature tests discover but do not run ignored DB cases.

## Verification

Pending native fixture startup, image architecture/version and builtin translated-error recovery. Adding the image is not test acceptance.
