# 2026-10-01 — T15 — private artifacts and structured console

- Status: in_progress (implementation preparation before pipeline acceptance)
- Role: B config_console
- Base revision: 6e9d7a4
- Claimed files: src/report/, this worklog.
- Dependency acceptance: real pipeline/recovery/verification remains in progress; no T15 completion claim.
- Guidance: Ponytail and RTK, concrete stdlib files/writers and existing serde.

## Reference research (before coding)

| Query / prefix | Pin and original implementation/tests read | Finding and chosen adaptation |
| --- | --- | --- |
| private artifacts reject lossless stdout / project-my2pg | 6e9d7a4 docs/config-and-cli.md and testing-and-performance.md:61; full contracts read | Exclusive private run directory, safe IDs, mandatory artifact I/O failures, JSON stdout; assert actual filesystem failure and byte fidelity |
| is_hidden / ref-indicatif | 53cf51c6da16789f9a29a7a1cfdf8e364dcb0868 src/draw_target.rs:136, tests:752–840 | Terminal/hidden target, stderr and width handling. Use stdlib IsTerminal and a bounded throttled stderr display; plain redirects never cursor-control, JSON never progress |
| reject-data / project-pgloader | 231ab86778ca5ffd7de40878714760c8b4860cdf src/utils/reject.lisp:11 and state.lisp:61; read full reject implementation | Appends encoded row and raw error condition with database/table-derived paths. Reject path derivation and raw error output; retain exact COPY bytes with typed locator/offset sidecar, safe generated IDs, sync errors fatal |

No source code copied. Float reject values use exact IEEE754 bits; bytes explicit hex, avoiding serde_json nonfinite-to-null loss.

## Verification

Implemented RunArtifacts private exclusive creation, exclusive plan, atomic durable report replacement, exact COPY byte + offset sidecars, typed raw rejects with bytes/IEEE bit fidelity, synchronous flush/sync and sticky reject failures. Console accepts structured ordinary events and a separate final RunReport outcome envelope; JSON stdout is separate from plain/TTY stderr, untrusted control characters visible, secret URL/assignments redacted, progress throttled to 4 Hz and metadata estimates never converted to percent-complete.

Nine report tests passed with cargo test --lib report:: on Rust 1.99.0. Tests inspect actual filesystem bytes/modes/offsets, preserve unsigned maximum/zero date/negative duration/nonfinite IEEE bits, verify exclusive dirs and report replacement, induce real filesystem failure and actual closed Unix socket write failure, distinguish JSON/plain output, preserve quiet final outcome, and throttle TTY estimates. No reference tests executed as evidence for our behavior.

Clippy initially found one owned collapsible_if after final-outcome addition; fixed. It also found two coordinator-owned transient convert/plan warnings, communicated rather than editing their files. Formatting applied only to owned config/cli/report files.

Full T15 gates remain open: production pipeline persistence/recovery, rejection-limit/accounting and failure propagation, real CLI JSON/plain/TTY/NO_COLOR/quiet/verbose subprocess cases, cancellation/closed-pipe shutdown, database error redaction audit and throughput metrics including encoded bytes. Coordinator must run full tests and lint after integration. No fake-completion claim.

Final API includes Console::outcome(&RunReport), emitting a version-1 outcome JSON envelope with effective exit code and full report, or an ASCII human final summary. Counts/status/verification survive narrow-terminal wrapping, and report paths remain complete. Ordinary event kinds are start/phase/progress/table_complete; table completion removes it from the active display. Final report diagnostics get defense-in-depth redaction; database classification must still exclude raw values and DETAIL upstream. All nine report tests passed again. JSON documents and hex records use temporary 16 KiB stdlib buffers; serde_json 1.0.151 collect_str implementation was inspected and streams the Display value rather than allocating the full hex string.

Latest all-target clippy was blocked by a coordinator/shared-contract update: postgres/mod.rs:404 lacked new TargetTable privilege fields. This is a different agent's in-progress code, reported for integration; no report lint failure is claimed fixed by unrelated state. T15 stays in progress.

## Real terminal acceptance continuation — research before coding

Root renewed an exact lease on src/report/console.rs, new tests/cases/t15_console.rs and this worklog. Root owns test registration, terminal dependency publication, table-scoped event counters and acknowledged COPY-byte metrics. Throughput acceptance remains open until those metrics are published and independently proved.

XERJ project-my2pg query "TTY NO_COLOR verbose width" returned config-and-cli.md:61 and this worklog:1 at indexed66c92db30cc1ba0a3ecdd9c0ac3ae4fe253f270f. Read original config-and-cli.md:93–116 and UX requirements; read the entire current console plus its adjacent tests and actual CLI pipeline/closed-output cases. Current synthetic forced-TTY tests do not prove stderr terminal detection or real dimensions; Args.verbose is unused, narrow widths clamp to20, and human outcomes omit unresolved/indeterminate row counts. The pipeline presently attaches aggregate run counters to a named table; root must correct that scope rather than the renderer fabricating per-table totals.

XERJ ref-indicatif query "term width columns" returned draw_target.rs:841 and901 at53cf51c6da16789f9a29a7a1cfdf8e364dcb0868. Read original hidden-target/width methods:136–167 and adjacent draw_to_term_narrower_than_its_content test:879–948, including0/1-column cases. Adapt safe terminal sizing, explicit redirect behavior, bounded throttling and width-aware ASCII output; retain exact names in artifacts/JSON. No reference code copied.

Installed/locked rustix1.1.5 (Cargo.lock checksum891efababe418670775f199f0d233d84843c227a0949a883ce15b37c78d6629d) provides safe termios::tcgetwinsize<Fd:AsFd>:termios/tc.rs:50, Winsize.ws_col in types.rs:1436; read original implementation and feature termios=[] in Cargo.toml:138. No packaged adjacent termios tests were found by the local search. Proposed Unix direct dependency publication to root; wait for it before compiling the dimension call. Use nonzero stderr dimensions ahead of COLUMNS fallback, without shelling out or authoring unsafe ioctl bindings.

Independent actual process acceptance will use Python stdlib pty/fcntl/termios/select with a deadline and child cleanup, setting real window dimensions and capturing stdout separately from stderr. Cases cover TTY/redirect, narrow windows, NO_COLOR and color policies, progress policies, quiet final status, verbose redacted diagnostics and JSON Lines final-outcome identity. Conversion-failure runs create actual reports with unresolved rows; indeterminate final rendering has a separate typed-report check until the coordinator's commit-fault process path is available. Existing closed-output/signal pipeline tests remain their own runtime evidence.

## Implemented continuation and acceptance

The coordinator published the Unix rustix1.1.5 termios dependency and shared serde-default metrics: RunEvent.committed_bytes, TableReport.committed_bytes/copy_elapsed_millis. Named events now carry table-scoped counts and COPY elapsed time; run events aggregate. These byte counts mean acknowledged PostgreSQL COPY-text bytes only, excluding rejects/retransmission/wire overhead/unacknowledged commits. The renderer consumes those fields directly, labels COPY-text-bytes/s explicitly and defines zero-duration rates as0. Final human summaries show total COPY-text bytes, unresolved and indeterminate rows; verbose output adds per-table source/target identities, status/accounting, consistency and per-table COPY-text byte rates. Diagnostics remain redacted and visible in quiet mode, with color policy applied to real final execution errors.

Actual stderr window dimensions take precedence over COLUMNS fallback. A1-column terminal is supported without an artificial20-column minimum; summaries wrap and ordinary lines truncate safely, while the usable artifact path stays complete. No shell dimension probe, unsafe ioctl wrapper, added framework or raw database-error logging.

Focused command on coordinator-owned fixture1d68940b1228:

    rtk run 'set -a; . my2pg/target/integration/my2pg-mysql84-pg16-1d68940b1228/env.sh; set +a; my2pg/bin/cargo test --locked --test integration t15_console -- --include-ignored --nocapture'

Result:5 passed,0 failed,0 ignored in2.16s (session48523). Four ignored database/process suites plus one ordinary typed-outcome case were explicitly included:

- Real PTY widths12 and1 trigger stderr cursor redraw, retain ASCII output and respect actual dimensions with COLUMNS absent. No estimate-based100% claim.
- Redirected plain output never cursor-controls, including --progress always; quiet keeps the final summary; verbose produces per-table detail; quiet JSON produces only versioned stdout JSON Lines and one matching final report envelope.
- Real conversion failures return1 with persisted unresolved=1/committed=0; PTY auto color, forced color, never color, redirected auto color and NO_COLOR overriding forced color all behave correctly. --progress never suppresses cursor redraw. Fixture password substrings and URLs are absent from both streams; raw invalid row contents are absent.
- Two-table actual JSON events prove table-scoped counts and independent acknowledged-byte expectations: all_bytes row518 bytes; keyless five rows29 bytes; report total547 bytes. Expectations are derived from fixed fixture text/COPY escaping, without calling the production encoder.
- A typed indeterminate outcome preserves unresolved/indeterminate accounting and exit1 in human/JSON rendering. This is renderer evidence; actual uncertain-COMMIT accounting belongs to the coordinator's fault tests.

Existing report console unit cases plus a new explicit42-byte/100ms→420 COPY-text-bytes/s and zero-time case pass6/6. Strict all-target Clippy, owned rustfmt --check and git diff --check pass after final changes. Initial process run passed4/5 but the redirected assertion expected an unbroken key across an intentionally wrapped final summary; fixed the test to normalize line breaks, not the renderer. The owned leftover schema/workspace from that failed assertion was identified and released; all subsequent successful cases clean their own schemas/workspaces. Root's shared fixture stays running under root ownership.

T15 remains in_progress until its T11/T13 dependencies and integrated recovery/artifact fault gates close. These process/metric tests are not a throughput benchmark, a cross-platform terminal matrix, or full-content verification. The coordinator owns final full-suite acceptance and shared-index refresh.
