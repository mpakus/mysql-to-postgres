# T15 — child worker integration preparation

Coordinator owns Cargo/features, internal CLI dispatch, resource/shared integration and registration. F owns concrete artifact_io module, minimal report/mod.rs integration and its native tests. C owns the pipeline panic increment; recovery/actor integration will require a separate agreed lease. No artifact durability/shutdown gate is accepted by this preparation.

## Reference research before feature change

XERJ project-my2pg `spawn_blocking bounded artifact child` at2b05730 returns the bounded-artifact proposal. Read that worklog, the complete later protocol/resource proposal, actual Cargo.toml, report synchronous writer and main.rs dispatch, plus adjacent writer tests. Reference research in F's worklogs includes pinned pgloader/dmt-rs original writer and bounded queue sources/tests; their missing durable acknowledgements are explicitly rejected.

Tokio1.53.1 is the locked primary implementation. Read registry src/process/mod.rs from_std definitions, Cargo feature declarations and adjacent tests/process_raw_handle.rs. std ChildStdin/ChildStdout convert to Tokio async pipe handles with the process feature; io-util provides async extension methods. The attempted version-specific docs.rs pages were inaccessible through the web tool, so actual locked source is the authoritative API evidence. Enable only those existing dependency features; do not add another subprocess framework. The process-global exact-owned std Child slot/reaper must survive Tokio runtime drop; kill_on_drop alone is not termination acknowledgement.

F's exact-count plus bounded serialization policy is approved in principle: a cap is an operational maximum, not a claim every uncapped model string fits. Oversized publication fails honestly and retains the last confirmed generation. Run-lifetime artifact memory reservation precedes table admission; schema-only cannot silently exempt actual artifact buffers. Concrete OwnedRejectBudget/Ticket replaces the borrowed actor-incompatible ticket during later recovery integration; one shared atomic budget and authoritative ACK ledger must remain.

Internal binary worker dispatch and source-test registration follow only once the concrete serve/facade API lands. No public CLI fault toggle or database credentials are passed to the writer. Record actual API and runtime proofs in subsequent entries.
