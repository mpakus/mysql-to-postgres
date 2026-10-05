use my2pg::config;
use std::path::Path;

const MAX_INPUT_BYTES: usize = 2 * 1024;
const MAX_CASES: usize = 512;
const MINIMAL: &str = "version = 1\n[source]\nurl_env = 'MY2PG_SOURCE'\nconsistency = 'frozen'\n[target]\nurl_env = 'MY2PG_TARGET'\nschema = 'legacy'\n";

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

#[test]
fn bounded_toml_configuration_fuzz_inputs_never_panic() {
    let directory = Path::new("/tmp/my2pg-config-fuzz");
    assert!(MINIMAL.len() <= MAX_INPUT_BYTES);
    let _ = config::parse(MINIMAL, directory);

    // Lossy UTF-8 expansion can triple each input byte, so bound raw samples
    // to one quarter of the parser's byte ceiling.
    let mut state = 0x4d59_3270_6746_555a_u64;
    for case in 0..MAX_CASES / 2 {
        let raw_len = (next(&mut state) as usize) % (MAX_INPUT_BYTES / 4 + 1);
        let mut bytes = Vec::with_capacity(raw_len);
        for _ in 0..raw_len {
            bytes.push(next(&mut state) as u8);
        }
        let input = String::from_utf8_lossy(&bytes);
        assert!(input.len() <= MAX_INPUT_BYTES, "case {case}");
        let _ = config::parse(&input, directory);
    }

    // Keep half the corpus structurally valid TOML so deserialization and
    // semantic validation see varied types and boundary values too.
    let fields = [
        "table_workers",
        "rows_per_range",
        "max_key_span",
        "max_rejected_rows",
        "reset_sequences",
        "mode",
    ];
    let values = [
        "0",
        "-1",
        "18446744073709551616",
        "true",
        "'invalid-mode'",
        "[]",
        "{}",
        "'safe-value'",
    ];
    for case in 0..MAX_CASES / 2 {
        let field = fields[(next(&mut state) as usize) % fields.len()];
        let value = values[(next(&mut state) as usize) % values.len()];
        let input = format!("{MINIMAL}[migration]\n{field} = {value}\n");
        assert!(input.len() <= MAX_INPUT_BYTES, "case {case}");
        let _ = config::parse(&input, directory);
    }
}
