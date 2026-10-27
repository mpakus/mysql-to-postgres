use my2pg::{
    model::{ColumnPlan, RawValue, ValueKind},
    verify::canonical::{CanonicalError, mysql_row, postgres_text_row},
};

fn column(kind: ValueKind, source_type: &str, target_type: &str) -> ColumnPlan {
    ColumnPlan {
        source_name: "value".into(),
        target_name: "value".into(),
        source_type: source_type.into(),
        target_type: target_type.into(),
        kind,
        nullable: true,
        default_sql: None,
        identity: false,
        generated_expression: None,
        copy: true,
        transform: None,
        charset: Some("utf8mb4".into()),
        enum_labels: Vec::new(),
        set_labels: Vec::new(),
        comment: None,
    }
}

#[test]
fn resolved_integer_decimal_text_and_boolean_values_match_postgres_text() {
    let columns = vec![
        column(ValueKind::SignedInteger, "int", "integer"),
        column(
            ValueKind::UnsignedInteger,
            "bigint unsigned",
            "numeric(20,0)",
        ),
        column(ValueKind::Decimal, "decimal", "numeric(30,8)"),
        column(ValueKind::Text, "varchar", "text"),
        column(ValueKind::Boolean, "tinyint", "boolean"),
    ];
    let source = mysql_row(
        &columns,
        &[
            RawValue::Int(-42),
            RawValue::UInt(u64::MAX),
            RawValue::Bytes(b"0012.3400".to_vec()),
            RawValue::Bytes("café".as_bytes().to_vec()),
            RawValue::Int(1),
        ],
    )
    .unwrap();
    let target = postgres_text_row(
        &columns,
        &[
            Some("-42"),
            Some("18446744073709551615"),
            Some("12.34"),
            Some("café"),
            Some("t"),
        ],
    )
    .unwrap();
    assert_eq!(source, target);

    // Independent wire oracle for one signed-integer field: type tag, presence,
    // 16-byte big-endian payload length, and i128 two's-complement bytes.
    assert_eq!(
        mysql_row(&columns[..1], &[RawValue::Int(-42)]).unwrap(),
        [
            vec![1, 1],
            (16u64).to_be_bytes().to_vec(),
            (-42i128).to_be_bytes().to_vec()
        ]
        .concat()
    );
}

#[test]
fn null_empty_and_approved_text_transformations_are_distinct_and_stable() {
    let text = column(ValueKind::Text, "varchar", "text");
    assert_eq!(
        mysql_row(std::slice::from_ref(&text), &[RawValue::Null]).unwrap(),
        postgres_text_row(std::slice::from_ref(&text), &[None]).unwrap()
    );
    assert_ne!(
        mysql_row(std::slice::from_ref(&text), &[RawValue::Bytes(Vec::new())]).unwrap(),
        postgres_text_row(std::slice::from_ref(&text), &[None]).unwrap()
    );
    assert_eq!(
        mysql_row(std::slice::from_ref(&text), &[RawValue::Bytes(Vec::new())]).unwrap(),
        postgres_text_row(std::slice::from_ref(&text), &[Some("")]).unwrap()
    );

    let mut trimmed = text;
    trimmed.transform = Some("right-trim".into());
    assert_eq!(
        mysql_row(
            std::slice::from_ref(&trimmed),
            &[RawValue::Bytes(b"tail \t".to_vec())]
        )
        .unwrap(),
        postgres_text_row(std::slice::from_ref(&trimmed), &[Some("tail")]).unwrap()
    );
}

#[test]
fn text_canonicalization_preserves_logical_controls_and_literal_backslashes() {
    let text = column(ValueKind::Text, "text", "text");
    let logical = b"tab\tline\nreturn\rslash\\backspace\x08formfeed\x0c\\t\\n\\N";
    let source = mysql_row(
        std::slice::from_ref(&text),
        &[RawValue::Bytes(logical.to_vec())],
    )
    .unwrap();
    let target = postgres_text_row(
        std::slice::from_ref(&text),
        &[Some(std::str::from_utf8(logical).unwrap())],
    )
    .unwrap();
    assert_eq!(source, target);
    let mut expected = vec![4, 1];
    expected.extend_from_slice(&(logical.len() as u64).to_be_bytes());
    expected.extend_from_slice(logical);
    assert_eq!(source, expected);

    let actual_tab = mysql_row(
        std::slice::from_ref(&text),
        &[RawValue::Bytes(b"\t".to_vec())],
    )
    .unwrap();
    let literal_backslash_t =
        postgres_text_row(std::slice::from_ref(&text), &[Some("\\t")]).unwrap();
    assert_ne!(actual_tab, literal_backslash_t);
}

#[test]
fn malformed_or_unreviewed_types_are_explicitly_rejected_without_values() {
    let decimal = column(ValueKind::Decimal, "decimal", "numeric(10,2)");
    let error =
        postgres_text_row(std::slice::from_ref(&decimal), &[Some("secret1e9")]).unwrap_err();
    assert!(matches!(
        error,
        CanonicalError::InvalidText(ValueKind::Decimal)
    ));
    assert!(!error.to_string().contains("secret"));

    let boolean = column(ValueKind::Boolean, "tinyint", "boolean");
    assert!(matches!(
        postgres_text_row(std::slice::from_ref(&boolean), &[Some("yes")]),
        Err(CanonicalError::InvalidText(ValueKind::Boolean))
    ));

    for kind in [ValueKind::Float, ValueKind::Json, ValueKind::Date] {
        let unsupported = column(kind.clone(), "source", "target");
        assert!(matches!(
            mysql_row(std::slice::from_ref(&unsupported), &[RawValue::Null]),
            Err(CanonicalError::UnsupportedKind(actual)) if actual == kind
        ));
    }
}

#[test]
fn binary_canonicalization_preserves_bytes_null_empty_and_approved_transforms() {
    let all_bytes: Vec<u8> = (0..=255).collect();
    let hex: String = (0..=255).map(|byte| format!("{byte:02X}")).collect();
    let mut expected = vec![6, 1];
    expected.extend_from_slice(&256u64.to_be_bytes());
    expected.extend_from_slice(&all_bytes);
    for transform in [
        None,
        Some("binary-to-bytea"),
        Some("byte-vector-to-bytea"),
        Some("bytes-to-pg-bytea"),
        Some("hex-to-bytea"),
    ] {
        let mut binary = column(ValueKind::Binary, "blob", "bytea");
        binary.transform = transform.map(str::to_string);
        let columns = std::slice::from_ref(&binary);
        let value = if transform == Some("hex-to-bytea") {
            hex.as_bytes().to_vec()
        } else {
            all_bytes.clone()
        };
        assert_eq!(
            mysql_row(columns, &[RawValue::Bytes(value)]).unwrap(),
            expected
        );
        assert_eq!(postgres_text_row(columns, &[Some(&hex)]).unwrap(), expected);
        assert_eq!(mysql_row(columns, &[RawValue::Null]).unwrap(), vec![6, 0]);
        assert_eq!(
            mysql_row(columns, &[RawValue::Bytes(vec![])]).unwrap(),
            [vec![6, 1], 0u64.to_be_bytes().to_vec()].concat()
        );
        assert_eq!(
            postgres_text_row(columns, &[Some("")]).unwrap(),
            [vec![6, 1], 0u64.to_be_bytes().to_vec()].concat()
        );
    }
    let mut binary = column(ValueKind::Binary, "blob", "bytea");
    for invalid in ["0", "0x00", "\\x00", "gg", "00 secret", "é"] {
        let error = postgres_text_row(std::slice::from_ref(&binary), &[Some(invalid)]).unwrap_err();
        assert!(matches!(
            error,
            CanonicalError::InvalidText(ValueKind::Binary)
        ));
        assert!(!error.to_string().contains(invalid));
    }
    binary.transform = Some("right-trim".into());
    assert!(matches!(
        mysql_row(&[binary], &[RawValue::Null]),
        Err(CanonicalError::UnsupportedKind(ValueKind::Binary))
    ));
}

#[test]
fn integer_and_row_width_failures_are_checked() {
    let integer = column(ValueKind::SignedInteger, "int", "integer");
    assert!(matches!(
        postgres_text_row(std::slice::from_ref(&integer), &[Some("1e2")]),
        Err(CanonicalError::InvalidText(ValueKind::SignedInteger))
    ));
    assert!(matches!(
        mysql_row(std::slice::from_ref(&integer), &[]),
        Err(CanonicalError::RowWidth)
    ));
}
