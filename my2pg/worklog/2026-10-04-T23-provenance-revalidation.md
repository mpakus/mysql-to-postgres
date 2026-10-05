# 2026-10-04 — T23 fixture and dependency-provenance revalidation

- Scope: revalidate the machine-checkable portion of the release provenance gate on the current dirty source tree; do not present this as legal advice or qualified license approval.
- Project HEAD: `db28c459a8a3caf45b07e9e72bcbc9fd3194c762`. Current project index is refreshed separately after this entry; reference pins are unchanged.

## Evidence

- `rtk test python3 tests/support/fixture_manifest.py` — passed: 13 `.load` files, 18 MySQL parser cases, 19 assertion cases, 65 case IDs; fixture pin and source hashes verified.
- `rtk test env CARGO_ABOUT=/private/tmp/cargo-about-extract/cargo-about-0.8.4-aarch64-apple-darwin/cargo-about CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/check-third-party-notices.sh --check` — exit 0 against the current lockfile and pinned generator. Cargo emitted only its existing missing documentation/homepage/repository metadata warning.
- `rtk test env PYTHONPYCACHEPREFIX=/tmp/my2pg-package-pycache python3 -m unittest tests.support.test_package -v` — 3 passed, checking included provenance/notices, archive exclusions, and packaged Markdown links.
- The latest two locked/offline Cargo source-package builds each compiled the extracted 223-file archive and matched SHA-256 `60e469754de9861fddb011467974f8e8381925f680d8e1ed64b974eff0c7247f`; see [current package evidence](2026-10-03-T23-current-source-package.md#2026-10-04-package-refresh-after-parser-review-regressions).
- Subsequent invalid-UTF-8 parser review changes were packaged and extracted successfully twice; both archive hashes match `2b64112a436c699cd96c3daeaaf1110d905f5cc1bac509624f93b01ef1e9fbff`. See [latest package evidence](2026-10-03-T23-current-source-package.md#2026-10-04-package-refresh-after-invalid-utf-8-review-fix).
- The table-local phase-event clock fix was packaged and extracted successfully twice; both archive hashes match `6dc50668af7f1553d8ecddfb3a684463495c042a14c5b40aac6d894df6bab7fd`. See [latest package evidence](2026-10-03-T23-current-source-package.md#2026-10-04-package-refresh-after-phase-event-clock-fix).

## Remaining review

These checks establish that the fixture manifest, pinned fixture inputs, generated notice bundle, package boundary and local source archive are internally consistent. They do not decide whether every dependency license is compatible with the intended distribution, nor whether every upstream fixture attribution is sufficient. The release checklist still requires qualified human compatibility and attribution review, native/distributed platform artifacts, and hosted CI evidence.
