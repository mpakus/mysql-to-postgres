# T23 — current source-package verification

This verifies the current dirty worktree's source package after the T15 test synchronization change. It does not claim a published release artifact or a reproducible clean build.

- `rtk run './bin/cargo fmt --all -- --check'` — passed.
- `rtk run './bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings'` — passed.
- `rtk run './bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1'` — passed: library 126/3 ignored, binary 1, contracts 15, driver 1/11 ignored, importer 18, integration 17/142 ignored, NLS 0/1 ignored.
- `rtk run 'env CARGO_ABOUT=/private/tmp/cargo-about-extract/cargo-about-0.8.4-aarch64-apple-darwin/cargo-about TMPDIR=/private/tmp CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/check-third-party-notices.sh --check'` — passed; Cargo emitted only the existing missing package homepage/repository/documentation metadata warning.
- `rtk run 'env CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/cargo package --allow-dirty --locked --offline'` — passed; Cargo packaged 220 files (2.5 MiB unpacked, 532.1 KiB compressed) and verified the extracted package compiled.
- Source archive: `target/package/my2pg-0.1.0.crate`; SHA-256 `30f8e6368351912502c1d383d96937dc8f9f04523651a2bcebadcc758e7e660d`.
- No database or container test ran in this package check. Native Linux/AMD64 execution, clean-checkout reproducibility, published checksums, and human dependency-license/fixture-attribution review remain open release gates.

## 2026-10-03 coordinator revalidation

The T22 applicability audit changed the current dirty source snapshot after the earlier hash above. Revalidated the current snapshot at HEAD `db28c459a8a3caf45b07e9e72bcbc9fd3194c762` (dirty tree):

- `rtk test ./bin/cargo test --offline --locked --all-targets --all-features -- --test-threads=1` — passed. Target summaries: library 126 passed/3 ignored; binary 1; contracts 15; driver 1 passed/11 ignored; importer 18; integration 17 passed/142 ignored; NLS 0 passed/1 ignored. Ignored database cases were listed only by compilation and were not run.
- `rtk test ./bin/cargo fmt --all -- --check` — passed.
- `rtk test ./bin/cargo clippy --offline --locked --all-targets --all-features -- -D warnings` — passed.
- `rtk test env CARGO_ABOUT=/private/tmp/cargo-about-extract/cargo-about-0.8.4-aarch64-apple-darwin/cargo-about CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/check-third-party-notices.sh --check` — passed; Cargo printed only the missing package homepage/repository/documentation metadata warning.
- Ran the same locked/offline `bin/cargo package --allow-dirty` command twice. Both runs verified the extracted package, contained 220 files (2.6 MiB unpacked, 541.3 KiB compressed), and produced SHA-256 `bf1833a0d80a825bfe1ac22e3aea0363fb392842f258b749ee691ba1fc578f90`.
- This is repeatability evidence for the current dirty macOS ARM64 source archive. It does not replace clean-checkout, distributed binary/container, native Linux, hosted CI, or qualified license and fixture-attribution gates. No database/container tests ran in this revalidation.

## 2026-10-03 coordinator package refresh after T22 CI edits

- Repeated `rtk test ./bin/cargo package --allow-dirty --locked --offline` twice after the current T22 test/documentation/CI changes. Both runs packaged and compiled the extracted 220-file package (2.6 MiB unpacked, 541.4 KiB compressed).
- Both archives have SHA-256 `374b7dafbcaf977faa1fe7254a28f7fa537370bcbd127e8b1d1685a08cf5965c`.
- Re-ran `rtk test env CARGO_ABOUT=/private/tmp/cargo-about-extract/cargo-about-0.8.4-aarch64-apple-darwin/cargo-about CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/check-third-party-notices.sh --check`; it passed with only the existing package metadata warning.
- This current-tree package remains dirty-checkout, macOS ARM64 evidence. Native Linux/AMD64, hosted Actions, publication, and qualified license/fixture attribution review remain open.

## 2026-10-03 package refresh after T21/T22 acceptance fixes

- Repeated `rtk test ./bin/cargo package --allow-dirty --locked --offline` twice after adding fail-closed T21 phase comparability and platform-aware T22 runnable-case enforcement.
- Both runs verified the extracted 220-file package (2.6 MiB unpacked, 542.5 KiB compressed) and produced SHA-256 `8ebe8812a25d89d67cd976bbc6c0524cdfa212de07afdd215d3be28a3e1ba19f`.
- This remains dirty-checkout package reproducibility evidence on macOS ARM64, not native Linux/AMD64 distribution, hosted CI, published artifact, or qualified legal review.

## 2026-10-03 package refresh after T22 NLS inventory correction

- Repeated `rtk run 'env CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/cargo package --allow-dirty --locked --offline'` three times after changing the packaged Compose guide.
- Every build verified the extracted 220-file package (2.6 MiB unpacked, 542.8 KiB compressed). The second and third archives matched SHA-256 `8ee065e6db21b665bbe5a6243be0d85152f31fe892aac41230ef98f1948d7a96`.
- This current archive supersedes the prior `8ebe8812...` hash above. It remains dirty-checkout macOS ARM64 package evidence; native Linux/AMD64, hosted CI, publication, and qualified license/fixture attribution gates remain open.

## 2026-10-03 package refresh after T21 local release-adapter fix

- Repeated `rtk run 'bin/cargo package --allow-dirty --locked --offline'` twice after changing `tests/performance/runner.py` and its regression tests. Both runs verified the extracted 220-file package (2.6 MiB unpacked, 543.0 KiB compressed).
- Both archives have SHA-256 `a56d71de69bc18b6fc6c3370fca501786bba66310c77e54a9f0fdc4a9d1251ab`; the archive contains both `tests/performance/runner.py` and `tests/performance/test_runner.py`.
- This supersedes the preceding package hash for the current dirty tree. It remains local macOS ARM64 source-package evidence; native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.

## 2026-10-04 package refresh after T22 parser and encoder regressions

- Repeated `rtk run 'env CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/cargo package --allow-dirty --locked --offline'` twice on the current dirty tree.
- Both runs packaged 223 files (2.6 MiB unpacked, 545.5 KiB compressed) and verified the extracted package compiled. Both archives had SHA-256 `e70095545e7aab515c1747dd3b51bfdd9016e697789cd6f766deca4d3f452a71`.
- `rtk run 'env TMPDIR=/private/tmp PYTHONPYCACHEPREFIX=/private/tmp/my2pg-package-pycache python3 -m unittest discover -s tests/support -p test_package.py -v'` — 3 passed; runtime/test/provenance inclusion, internal-material exclusions and packaged Markdown links all pass.
- The only Cargo warning is the existing absent documentation/homepage/repository metadata. This remains dirty-checkout macOS ARM64 source-package evidence; native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.

## 2026-10-04 package refresh after pgloader summary telemetry

- Repeated `rtk test ./bin/cargo package --allow-dirty --locked --offline` on the current dirty source. Each build packaged 223 files (2.6 MiB unpacked, 546.7 KiB compressed) and compiled the extracted archive.
- The second and third consecutive captured archive SHA-256 values matched: `bbe440bea1dde34c8ad674b038fa8004c757606cd45bb445d32d55d227b63630`.
- `rtk test env PYTHONPYCACHEPREFIX=/tmp/my2pg-package-pycache python3 -m unittest discover -s tests/support -p test_package.py -v` — 3 passed.
- `rtk test env CARGO_ABOUT=/private/tmp/cargo-about-extract/cargo-about-0.8.4-aarch64-apple-darwin/cargo-about CARGO_HOME=/Users/renatibragimov/www/pg/.toolchains/cargo RUSTUP_HOME=/Users/renatibragimov/www/pg/.toolchains/rustup PATH=/Users/renatibragimov/www/pg/.toolchains/cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin ./bin/check-third-party-notices.sh --check` — passed; only the existing package metadata warning was emitted.
- The package now includes the pgloader summary adapter and its tests. This is local dirty-checkout Darwin/ARM64 repeatability evidence; native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.

## 2026-10-04 package refresh after parser review regressions

- Repeated `rtk test ./bin/cargo package --allow-dirty --locked --offline` twice after adding malformed-JSON, missing-phase-total, and duplicate-COPY summary parser tests. Both runs compiled the extracted 223-file archive (2.6 MiB unpacked, 546.9 KiB compressed).
- Both captured archive SHA-256 values matched: `60e469754de9861fddb011467974f8e8381925f680d8e1ed64b974eff0c7247f`. `rtk test env PYTHONPYCACHEPREFIX=/tmp/t21-pycache python3 -m unittest tests.support.test_package -v` passed 3/3.
- This is local dirty-checkout Darwin/ARM64 packaging evidence. Native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.

## 2026-10-04 package refresh after invalid-UTF-8 review fix

- Repeated the locked/offline package build twice after the summary parser began catching `UnicodeDecodeError` and gained an invalid-UTF-8 regression. Both builds compiled the extracted 223-file archive (2.6 MiB unpacked, 546.9 KiB compressed).
- Both archive SHA-256 values matched: `2b64112a436c699cd96c3daeaaf1110d905f5cc1bac509624f93b01ef1e9fbff`. The package-boundary suite passed 3/3.
- This remains local dirty-checkout Darwin/ARM64 packaging evidence. Native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.

## 2026-10-04 package refresh after phase-event clock fix

- Repeated the locked/offline package build twice after adding the table-scoped phase-event regression. Each build verified the extracted 223-file archive (2.6 MiB unpacked, 547.1 KiB compressed).
- Both archive hashes matched: `6dc50668af7f1553d8ecddfb3a684463495c042a14c5b40aac6d894df6bab7fd`; the package-boundary suite passed 3/3.
- This remains local dirty-checkout Darwin/ARM64 packaging evidence. Native Linux/AMD64, hosted CI, published artifacts, and qualified license/fixture-attribution review remain open.
