use my2pg::{convert::encode_row_limited, model::*};

const MAX_CASES: usize = 512;
const MAX_ROW_BYTES: usize = 1024;
const TOKENS: &[&str] = &[
    "a", "Z", "0", "\\", "\t", "\n", "\r", "\x08", "\x0c", "\x0b", "\\N", "é", "雪", "😀",
];

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn column() -> ColumnPlan {
    ColumnPlan {
        source_name: "value".into(),
        target_name: "value".into(),
        source_type: "fixture".into(),
        target_type: "text".into(),
        kind: ValueKind::Text,
        nullable: true,
        default_sql: None,
        identity: false,
        generated_expression: None,
        copy: true,
        transform: None,
        charset: Some("utf8mb4".into()),
        enum_labels: vec![],
        set_labels: vec![],
        comment: None,
    }
}

fn table() -> TablePlan {
    TablePlan {
        id: "t_0000000000000000".into(),
        source_name: "fixture".into(),
        target_schema: "legacy".into(),
        target_name: "fixture".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        columns: vec![column(), column()],
        primary_key: vec![],
        structure: Default::default(),
    }
}

fn generated_text(state: &mut u64) -> String {
    let count = (next(state) as usize) % 48;
    let mut value = String::new();
    for _ in 0..count {
        value.push_str(TOKENS[(next(state) as usize) % TOKENS.len()]);
    }
    value
}

// Independent byte oracle for PostgreSQL COPY TEXT framing and escaping.
fn append_field(value: &str, output: &mut Vec<u8>) {
    for byte in value.bytes() {
        match byte {
            b'\\' => output.extend_from_slice(b"\\\\"),
            b'\t' => output.extend_from_slice(b"\\t"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            8 => output.extend_from_slice(b"\\b"),
            12 => output.extend_from_slice(b"\\f"),
            11 => output.extend_from_slice(b"\\v"),
            _ => output.push(byte),
        }
    }
}

#[test]
fn bounded_generated_copy_rows_match_independent_text_oracle_and_exact_limits() {
    let table = table();
    let mut state = 0x4d59_3270_434f_5059_u64;

    for case in 0..MAX_CASES {
        let first = generated_text(&mut state);
        let second = generated_text(&mut state);
        let second_is_null = case % 4 == 0;
        let values = [
            RawValue::Bytes(first.as_bytes().to_vec()),
            if second_is_null {
                RawValue::Null
            } else {
                RawValue::Bytes(second.as_bytes().to_vec())
            },
        ];

        let mut expected = Vec::new();
        append_field(&first, &mut expected);
        expected.push(b'\t');
        if second_is_null {
            expected.extend_from_slice(b"\\N");
        } else {
            append_field(&second, &mut expected);
        }
        expected.push(b'\n');
        assert!(expected.len() <= MAX_ROW_BYTES, "case {case}");

        let encoded = encode_row_limited(&table, &values, expected.len())
            .unwrap_or_else(|error| panic!("case {case}: {error}"));
        assert_eq!(encoded, expected, "case {case}");
        assert_eq!(encoded.capacity(), encoded.len(), "case {case}");
        assert!(
            encode_row_limited(&table, &values, expected.len() - 1).is_err(),
            "case {case} accepted a row one byte over its limit"
        );
    }
}
