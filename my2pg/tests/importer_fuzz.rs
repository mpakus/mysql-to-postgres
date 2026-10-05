use my2pg::config::import::{DefaultPolicy, ImportOptions, parse};

const MAX_CASES: usize = 512;
const MAX_INPUT_BYTES: usize = 2 * 1024;
const HEAD: &str = "LOAD DATABASE FROM mysql://reader@127.0.0.1/shop INTO postgresql://writer@127.0.0.1/warehouse ";

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn options() -> ImportOptions {
    ImportOptions {
        target_schema: Some("legacy".into()),
        default_policy: DefaultPolicy::My2pgReviewed,
        ..Default::default()
    }
}

fn exercise(input: &str) {
    assert!(input.len() <= MAX_INPUT_BYTES);
    if let Err(error) = parse(input, &options()) {
        assert!(error.span.start <= error.span.end, "{error}");
        assert!(error.span.end <= input.len(), "{error}");
    }
}

#[test]
fn bounded_generated_load_inputs_return_results_with_valid_error_spans() {
    let mut state = 0x4d59_3270_494d_5054_u64;

    // Arbitrary byte-derived strings exercise UTF-8 boundaries and tokenizer rejection.
    for case in 0..MAX_CASES / 2 {
        let raw_len = (next(&mut state) as usize) % 513;
        let mut bytes = Vec::with_capacity(raw_len);
        for _ in 0..raw_len {
            bytes.push(next(&mut state) as u8);
        }
        let input = String::from_utf8_lossy(&bytes);
        assert!(input.len() <= MAX_INPUT_BYTES, "case {case}");
        exercise(&input);
    }

    // Grammar-shaped mutations reach the closed clause parser and validators.
    let templates = [
        "WITH batch rows={value}",
        "WITH workers={value}",
        "INCLUDING ONLY TABLE NAMES MATCHING ~/{value}/",
        "ALTER TABLE NAMES MATCHING 'orders' RENAME TO '{value}'",
        "CAST type decimal to {value}",
        "CAST type decimal when {value} to numeric",
    ];
    let long_value = "x".repeat(768);
    let values = [
        "0",
        "1",
        "-1",
        "18446744073709551616",
        "text",
        "numeric(20,0)",
        "a+b",
        "[",
        "\\w+",
        "orders' WITH disable triggers",
        long_value.as_str(),
    ];
    for case in 0..MAX_CASES / 2 {
        let template = templates[(next(&mut state) as usize) % templates.len()];
        let value = values[(next(&mut state) as usize) % values.len()];
        let input = format!("{HEAD}{};", template.replace("{value}", value));
        assert!(input.len() <= MAX_INPUT_BYTES, "case {case}");
        exercise(&input);
    }
}
