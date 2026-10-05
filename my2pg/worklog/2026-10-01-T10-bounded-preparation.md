# 2026-10-01 — T10 preparation — bounded encoding and sequence limits

Status: in_progress. Coordinator renewed D lease for src/plan/, src/convert/, t06_plan acceptance and this preparation log. T10 fidelity remains gated by T09; this increment prepares T08's bounded pipeline. RTK and Ponytail remain active. Shared model/Cargo remain coordinator-owned.

## Reference research before coding

- XERJ project-my2pg query `max_row_bytes memory row workspace` returned validation.rs:181–240 and mysql/mod.rs:421–535 at project revision6e9d7a4+working snapshot. Read originals/adjacent tests: current budget reserves raw/encoded rows but existing encoder adds large per-column strings/JSON DOM. Need remove those transients before the budget is credible.
- XERJ ref-rust-postgres query `BufMut copy_in` at pin1084ca8f5b5302e161892f2fa40abf71b4060c10 returned copy_in.rs:181–220 and tests; read original copy sink and copy_in_error test739–762. Driver buffering remains separate from application workspace. Our converter returns one complete bounded COPY row; target transport observes finish/commit separately.
- XERJ project-pgloader query `copy encode bytea escape` at pin231ab86778ca5ffd7de40878714760c8b4860cdf returned cast.clj:61–88. Read original and adjacent copy_test.clj:25–50. Reference allocates a hex string then COPY escaping and ignores extra columns. Reject those behaviors; stream hex after exact escaped-length measurement, retain exact row arity.
- Initially investigated pinned Cargo serde_json1.0.151 src/de.rs:1102–1250,1910–1916 and src/read.rs:598–624,1025–1048: IgnoredAny lacks decoded NUL/surrogate pairing checks. Coordinator requested the smaller existing recursive Visitor/DeserializeSeed approach. Rejected a custom prescanner/duplicated grammar before implementation; final Visitor discards each map/sequence entry immediately, validates keys and values, and delegates Unicode/nesting/number syntax to serde. Read de.rs:937–955,1400–1453,2210–2277 and number.rs:653–700; parser can retain decoded-string scratch plus one arbitrary-precision number String, with no DOM. No reference implementation copied.
- Coordinator installed matching Rust1.99 rust-src; read compiler commitb940084d7eb6a299eb4bfeb8e34901bc051e7ac4, library/alloc/src/raw_vec/mod.rs:503–533. Growth is max(2*old_capacity,required), u8 minimum8; serde number String begins at16. Read serde read.rs:495–533,978–1007 for scratch extension and Unicode spare-four reserve. This proves a conservative retained capacity bound rather than assuming a multiplier. The earlier direct GitHub fetch of this compiler commit missed; installed source provided exact evidence.

## Chosen contracts

`encode_row_limited(table,values,maximum)` measures the escaped row and rejects oversize before allocation, then allocates exact output length. Large text/decimal/JSON data remain borrowed; binary hex streams directly; fixed scalar formatting stays on bounded stack. Existing encode_row delegates with unrestricted limit for compatibility. encode_value continues returning the shared unescaped owned text API for callers that explicitly need it.

Sequence expectations apply only proved default ascending increment1/offset1, min1/no-cycle, with target integer-type maximum and source next-value lower bound. Source next outside a smallint/integer/bigint sequence range blocks planning. Existing sequence metadata is not inferred; data-only identity sequencing remains an explicit unsupported/fidelity gate until inspected metadata proves compatibility.

## Workspace proof and practical limits

Let R be the admitted maximum row bytes. Large raw text/decimal/JSON fields are borrowed; bytea hex is written directly. Scalar representations use a fixed512-byte stack buffer. No per-row column list or full JSON tree is allocated. Measurement validates JSON once; parser buffers drop before the exact output buffer is allocated, and the emission pass reuses immutable inputs without revalidation.

For serde_json1.0.151, decoded-string scratch length never exceeds the original input field length; Unicode may request four spare bytes. When Rust1.99 grows a u8 vector, its old capacity is below the required capacity, so doubling gives less than twice the largest request. Scratch retained capacity is therefore at most max(8,2*(R+4)). The separately retained number String begins with16bytes and grows on token-byte pushes, so its retained capacity is at most max(16,2*R). `encoder_workspace_bytes(R)` returns the checked sum of these independently possible buffers; e.g. R100 gives408bytes. The Visitor also checks an owned number token's observed capacity against the pin-derived bound. All arithmetic overflows reject configuration/admission through the public helper.

Output size includes delimiters, final newline, SQLNULL marker and escaping expansion. Oversized large fields reject before decoder allocation; complete escaped size rejects before exact output reservation. No output Vec growth is needed. Caller raw storage and output R are accounted separately by the pipeline; using their sum with the workspace helper is conservative because parser workspace and output allocation occur in separate passes.

These are retained application/parser capacities, not a hard process-RSS bound. Allocator rounding/reallocation overlap, recursive parser/compiler stack, MySQL packet/decoder caches and PostgreSQL/TLS buffers require separate measurement. Source raw_values moves byte payloads but may briefly retain both MySQL Value and RawValue enum vectors; compiler allocation reuse is not a contract. F must reserve that overlap, and T13 retains the unaudited transport-memory gap explicitly.

## Verification so far

- Planner15/15 unit tests passed, including narrow sequence exhaustion and typed default identity maxima.
- Encoder8/8 unit tests passed, including exact encoded-size boundary, COPY escape expansion, Unicode bytes, NULL/delimiters, binary hex, huge exact JSON numbers, escaped NUL keys/values, literal backslash-u, surrogate pairs/rejection, malformed/trailing input, excessive nesting and wide arrays.
- `bin/cargo clippy --locked --all-targets -- -D warnings` passed before the final immutable-second-pass optimization; affected checks are rerunning.
- T06 real fixture now uses the bounded encoder. Fresh mandatory runtime and T08 actual CLI alpha are pending coordinator orchestration.

Reader `TablePlan.primary_key` remains source identity even if the target PRIMARY is explicitly omitted. The target verifier now derives expected PK from typed structure.indexes (E's lease); omission is not mistaken for source identity deletion.
