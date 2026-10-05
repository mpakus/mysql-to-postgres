use super::*;

fn column(kind: ValueKind, target: &str) -> ColumnPlan {
    ColumnPlan {
        source_name: "value".into(),
        target_name: "value".into(),
        source_type: "fixture".into(),
        target_type: target.into(),
        kind,
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
fn row(columns: Vec<ColumnPlan>) -> TablePlan {
    TablePlan {
        id: "t_0000000000000000".into(),
        source_name: "fixture".into(),
        target_schema: "legacy".into(),
        target_name: "fixture".into(),
        engine: "InnoDB".into(),
        is_view: false,
        estimated_rows: None,
        next_auto_increment: None,
        columns,
        primary_key: vec![],
        structure: Default::default(),
    }
}

#[test]
fn null_empty_literal_backslash_n_and_control_escapes_are_distinct() {
    let table = row(vec![column(ValueKind::Text, "text"); 4]);
    let encoded = encode_row(
        &table,
        &[
            RawValue::Null,
            RawValue::Bytes(vec![]),
            RawValue::Bytes(b"\\N".to_vec()),
            RawValue::Bytes(b"\t\n\r\\\x08\x0c".to_vec()),
        ],
    )
    .unwrap();
    assert_eq!(encoded, b"\\N\t\t\\\\N\t\\t\\n\\r\\\\\\b\\f\n");
}

#[test]
fn bytea_has_one_value_escape_and_two_copy_escapes() {
    let binary = column(ValueKind::Binary, "bytea");
    let bytes = RawValue::Bytes(vec![0, 1, 92, 9, 255]);
    assert_eq!(
        encode_value(&binary, &bytes).unwrap().as_deref(),
        Some("\\x00015c09ff")
    );
    assert_eq!(
        encode_row(&row(vec![binary]), &[bytes]).unwrap(),
        b"\\\\x00015c09ff\n"
    );
}

#[test]
fn ewkt_geometry_values_preserve_srid_and_null_and_reject_malformed_projection() {
    let geometry = column(ValueKind::Geometry, "geometry");
    assert_eq!(
        encode_value(
            &geometry,
            &RawValue::Bytes(b"SRID=4326;POINT(1 2)".to_vec())
        )
        .unwrap()
        .as_deref(),
        Some("SRID=4326;POINT(1 2)")
    );
    assert_eq!(encode_value(&geometry, &RawValue::Null).unwrap(), None);
    for malformed in [
        "POINT(1 2)",
        "SRID=;POINT EMPTY",
        "SRID=bad;POINT(1 2)",
        "SRID=0;",
    ] {
        assert!(
            encode_value(&geometry, &RawValue::Bytes(malformed.as_bytes().to_vec())).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn limited_rows_measure_escape_expansion_and_exact_boundaries() {
    let fixtures = [
        (
            row(vec![column(ValueKind::Text, "text")]),
            vec![RawValue::Bytes(b"\t\\\n".to_vec())],
            b"\\t\\\\\\n\n".to_vec(),
        ),
        (
            row(vec![column(ValueKind::Binary, "bytea")]),
            vec![RawValue::Bytes(vec![0, 255])],
            b"\\\\x00ff\n".to_vec(),
        ),
        (
            row(vec![column(ValueKind::Text, "text"); 2]),
            vec![RawValue::Null, RawValue::Bytes("😀".as_bytes().to_vec())],
            "\\N\t😀\n".as_bytes().to_vec(),
        ),
    ];
    for (table, values, expected) in fixtures {
        let encoded = encode_row_limited(&table, &values, expected.len()).unwrap();
        assert_eq!(encoded, expected);
        assert_eq!(encoded.capacity(), encoded.len());
        assert!(encode_row_limited(&table, &values, encoded.len() - 1).is_err());
    }
    assert!(encode_row_limited(&row(vec![]), &[], 0).is_err());
    assert_eq!(encode_row_limited(&row(vec![]), &[], 1).unwrap(), b"\n");
    assert!(encoder_workspace_bytes(usize::MAX).is_err());
    assert_eq!(encoder_workspace_bytes(100).unwrap(), 408);
}

#[test]
fn streaming_json_validates_keys_values_unicode_depth_and_exact_huge_numbers() {
    let table = row(vec![column(ValueKind::Json, "jsonb")]);
    for valid in [
        r#"{"wide":1844674407370955161500000000000000000000000000000000,"value":"\uD83D\uDE00"}"#,
        r#"{"literal":"\\u0000","null":null}"#,
    ] {
        let values = [RawValue::Bytes(valid.as_bytes().to_vec())];
        let expected = encode_row(&table, &values).unwrap();
        assert_eq!(
            encode_row_limited(&table, &values, expected.len()).unwrap(),
            expected
        );
        assert!(encode_row_limited(&table, &values, expected.len() - 1).is_err());
    }
    for invalid in [
        r#"{"\u0000":0}"#,
        r#"{"nested":["\u0000"]}"#,
        r#""\uD800""#,
        r#""\uDC00""#,
        r#"{"a":1} trailing"#,
    ] {
        assert!(
            encode_row_limited(
                &table,
                &[RawValue::Bytes(invalid.as_bytes().to_vec())],
                1024
            )
            .is_err()
        );
    }
    let deeply_nested = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    assert!(
        encode_row_limited(&table, &[RawValue::Bytes(deeply_nested.into_bytes())], 1024).is_err()
    );
    let many_values = format!("[{}]", vec!["1"; 10000].join(","));
    encode_row_limited(&table, &[RawValue::Bytes(many_values.into_bytes())], 65536).unwrap();
    assert!(encode_row_limited(&table, &[RawValue::UInt(u64::MAX)], 2).is_err());
}

#[test]
fn exact_unsigned_decimal_and_target_range_guards() {
    assert_eq!(
        encode_value(
            &column(ValueKind::UnsignedInteger, "numeric(20,0)"),
            &RawValue::UInt(u64::MAX)
        )
        .unwrap()
        .as_deref(),
        Some("18446744073709551615")
    );
    let decimal = column(ValueKind::Decimal, "numeric(30,8)");
    let value = RawValue::Bytes(b"1234567890123456789012.12345678".to_vec());
    assert_eq!(
        encode_value(&decimal, &value).unwrap().as_deref(),
        Some("1234567890123456789012.12345678")
    );
    assert!(encode_value(&decimal, &RawValue::Double(1.5)).is_err());
    assert!(encode_value(&decimal, &RawValue::Bytes(b"1e10".to_vec())).is_err());
    assert!(
        encode_value(
            &column(ValueKind::UnsignedInteger, "smallint"),
            &RawValue::UInt(65_535)
        )
        .is_err()
    );
}

#[test]
fn invalid_text_dates_and_row_width_fail_without_raw_data_in_diagnostics() {
    let text = column(ValueKind::Text, "text");
    for bytes in [vec![255], b"secret\0data".to_vec()] {
        let error = encode_value(&text, &RawValue::Bytes(bytes)).unwrap_err();
        assert!(!error.to_string().contains("secret"));
    }
    assert!(encode_row(&row(vec![text]), &[]).is_err());
    let date = RawValue::Date {
        year: 2023,
        month: 2,
        day: 29,
        hour: 0,
        minute: 0,
        second: 0,
        micros: 0,
    };
    assert!(encode_value(&column(ValueKind::Date, "date"), &date).is_err());
}

#[test]
fn negative_and_multi_day_time_remain_durations_and_bits_keep_width() {
    let time = RawValue::Time {
        negative: true,
        days: 34,
        hour: 22,
        minute: 59,
        second: 58,
        micros: 999_999,
    };
    assert_eq!(
        encode_value(&column(ValueKind::Interval, "interval(6)"), &time)
            .unwrap()
            .as_deref(),
        Some("-838:59:58.999999")
    );
    assert!(encode_value(&column(ValueKind::Interval, "time(6)"), &time).is_err());
    assert_eq!(
        encode_value(
            &column(ValueKind::Bit(8), "bit(8)"),
            &RawValue::Bytes(vec![5])
        )
        .unwrap()
        .as_deref(),
        Some("00000101")
    );
}

#[test]
fn explicit_transformed_null_still_respects_resolved_nullability() {
    let mut date = column(ValueKind::Datetime, "timestamp");
    date.transform = Some("zero-dates-to-null".into());
    let raw = RawValue::Date {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        micros: 0,
    };
    assert_eq!(encode_value(&date, &raw).unwrap(), None);
    date.nullable = false;
    assert!(encode_value(&date, &raw).is_err());
}

#[test]
fn float_widening_and_narrowing_preserve_binary_value() {
    let double = column(ValueKind::Float, "double precision");
    assert_eq!(
        encode_value(&double, &RawValue::Float(0.1))
            .unwrap()
            .as_deref(),
        Some("0.10000000149011612")
    );
    let real = column(ValueKind::Float, "real");
    assert_eq!(
        encode_value(&real, &RawValue::Float(0.1))
            .unwrap()
            .as_deref(),
        Some("0.1")
    );
    assert_eq!(
        encode_value(&real, &RawValue::Double(0.5))
            .unwrap()
            .as_deref(),
        Some("0.5")
    );
    for value in [
        RawValue::Double(0.1),
        RawValue::Double(f64::MAX),
        RawValue::Double(f64::NAN),
        RawValue::Float(f32::INFINITY),
    ] {
        assert!(encode_value(&real, &value).is_err());
    }
    assert!(encode_value(&double, &RawValue::UInt(u64::MAX)).is_err());
}

#[test]
fn source_decimal_domain_and_fractional_precision_are_enforced() {
    let decimal = column(ValueKind::Decimal, "numeric(65,30)");
    let exact = format!("{}.{}", "9".repeat(35), "9".repeat(30));
    assert_eq!(
        encode_value(&decimal, &RawValue::Bytes(exact.as_bytes().to_vec()))
            .unwrap()
            .as_deref(),
        Some(exact.as_str())
    );
    for text in [
        format!("{}.{}", "9".repeat(36), "9".repeat(30)),
        "1e1".into(),
        "1.".into(),
        format!("0.{}", "0".repeat(31)),
    ] {
        assert!(encode_value(&decimal, &RawValue::Bytes(text.into_bytes())).is_err());
    }
    let mut date = RawValue::Date {
        year: 2024,
        month: 2,
        day: 29,
        hour: 1,
        minute: 2,
        second: 3,
        micros: 123_000,
    };
    let timestamp = column(ValueKind::Datetime, "timestamp(3) without time zone");
    assert_eq!(
        encode_value(&timestamp, &date).unwrap().as_deref(),
        Some("2024-02-29 01:02:03.123000")
    );
    if let RawValue::Date { micros, .. } = &mut date {
        *micros = 123_001;
    }
    assert!(encode_value(&timestamp, &date).is_err());
    assert!(encode_value(&column(ValueKind::Date, "date"), &date).is_err());
    let outside = RawValue::Time {
        negative: false,
        days: 34,
        hour: 23,
        minute: 0,
        second: 0,
        micros: 0,
    };
    assert!(encode_value(&column(ValueKind::Interval, "interval(6)"), &outside).is_err());
}

#[test]
fn enum_ordinals_and_set_masks_preserve_empty_order_and_all64_bits() {
    let mut enumeration = column(ValueKind::Enum, "\"legacy\".\"fixture_enum\"");
    enumeration.enum_labels = vec!["".into(), "comma,label".into(), "quote\"slash\\".into()];
    assert_eq!(
        encode_value(&enumeration, &RawValue::UInt(1))
            .unwrap()
            .as_deref(),
        Some("")
    );
    assert_eq!(
        encode_value(&enumeration, &RawValue::UInt(2))
            .unwrap()
            .as_deref(),
        Some("comma,label")
    );
    assert!(encode_value(&enumeration, &RawValue::UInt(0)).is_err());
    assert!(encode_value(&enumeration, &RawValue::UInt(4)).is_err());
    enumeration.transform = Some("empty-string-to-null".into());
    assert_eq!(
        encode_value(&enumeration, &RawValue::UInt(1)).unwrap(),
        None
    );
    assert!(transformation_applied(&enumeration, &RawValue::UInt(1)).unwrap());
    assert!(encode_value(&enumeration, &RawValue::UInt(0)).is_err());
    let mut set = column(ValueKind::Set, "text[]");
    set.set_labels = (0..64).map(|i| format!("label{i}")).collect();
    set.set_labels[0] = "a\"\\b".into();
    assert_eq!(
        encode_value(&set, &RawValue::UInt(0)).unwrap().as_deref(),
        Some("{}")
    );
    assert_eq!(
        encode_value(&set, &RawValue::UInt((1 << 63) | 1))
            .unwrap()
            .as_deref(),
        Some("{\"a\\\"\\\\b\",\"label63\"}")
    );
    let table = row(vec![set]);
    let raw = [RawValue::UInt((1 << 63) | 1)];
    let encoded = encode_row(&table, &raw).unwrap();
    assert_eq!(encoded, b"{\"a\\\\\"\\\\\\\\b\",\"label63\"}\n");
    assert_eq!(
        encode_row_limited(&table, &raw, encoded.len()).unwrap(),
        encoded
    );
    assert!(encode_row_limited(&table, &raw, encoded.len() - 1).is_err());
}

#[test]
fn strict_charsets_and_named_transforms_stream_without_replacement() {
    let mut text = column(ValueKind::Text, "varchar(3)");
    text.charset = Some("latin1".into());
    assert_eq!(
        encode_value(&text, &RawValue::Bytes(vec![0x80, 0x81, 0xff]))
            .unwrap()
            .as_deref(),
        Some("€\u{81}ÿ")
    );
    text.charset = Some("iso-8859-1".into());
    assert_eq!(
        encode_value(&text, &RawValue::Bytes(vec![0x80]))
            .unwrap()
            .as_deref(),
        Some("\u{80}")
    );
    text.charset = Some("ascii".into());
    assert!(encode_value(&text, &RawValue::Bytes(vec![0x80])).is_err());
    text.charset = Some("utf8mb4".into());
    text.transform = Some("remove-null-characters".into());
    let raw = RawValue::Bytes(b"a\0b".to_vec());
    assert_eq!(
        encode_row_limited(&row(vec![text.clone()]), std::slice::from_ref(&raw), 3).unwrap(),
        b"ab\n"
    );
    assert!(transformation_applied(&text, &raw).unwrap());
    text.transform = Some("right-trim".into());
    let raw = RawValue::Bytes("a \u{2003}".as_bytes().to_vec());
    assert_eq!(
        encode_row_limited(&row(vec![text.clone()]), std::slice::from_ref(&raw), 2).unwrap(),
        b"a\n"
    );
    assert!(transformation_applied(&text, &raw).unwrap());
    let mut hex = column(ValueKind::Binary, "bytea");
    hex.transform = Some("hex-to-bytea".into());
    assert_eq!(
        encode_value(&hex, &RawValue::Bytes(b"0x00FF5c".to_vec()))
            .unwrap()
            .as_deref(),
        Some("\\x00ff5c")
    );
    for invalid in ["0", "gg", "0x1"] {
        assert!(encode_value(&hex, &RawValue::Bytes(invalid.as_bytes().to_vec())).is_err());
    }
    let mut ip = column(ValueKind::Text, "inet");
    ip.transform = Some("int-to-ip".into());
    assert_eq!(
        encode_value(&ip, &RawValue::UInt(0x7f000001))
            .unwrap()
            .as_deref(),
        Some("127.0.0.1")
    );
    assert!(encode_value(&ip, &RawValue::UInt(u64::MAX)).is_err());
    let mut bit = column(ValueKind::Boolean, "boolean");
    bit.source_type = "bit(1)".into();
    bit.transform = Some("bits-to-boolean".into());
    assert_eq!(
        encode_value(&bit, &RawValue::Bytes(vec![1]))
            .unwrap()
            .as_deref(),
        Some("true")
    );
    assert!(encode_value(&bit, &RawValue::Bytes(vec![2])).is_err());
}

#[test]
fn jsonb_numeric_domain_is_checked_without_expanding_exponents() {
    let json = column(ValueKind::Json, "jsonb");
    for valid in [
        "1e131071",
        "1e-16383",
        r#"{"$serde_json::private::Number":"1e999999999999999999999999"}"#,
        r#"["1e999999999999999999999",0]"#,
    ] {
        assert!(
            encode_value(&json, &RawValue::Bytes(valid.as_bytes().to_vec())).is_ok(),
            "{valid}"
        );
    }
    for invalid in [
        "1e131072",
        "1e-16384",
        "1e99999999999999999999999",
        "1e-999999999999999999999999",
    ] {
        assert!(
            encode_value(&json, &RawValue::Bytes(invalid.as_bytes().to_vec())).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn all_named_transform_effects_distinguish_semantic_changes_from_representation() {
    for name in ["byte-vector-to-bytea", "bytes-to-pg-bytea"] {
        let mut binary = column(ValueKind::Binary, "bytea");
        binary.transform = Some(name.into());
        for bytes in [vec![], vec![0, 255]] {
            let raw = RawValue::Bytes(bytes);
            assert!(!transformation_applied(&binary, &raw).unwrap());
            assert!(encode_value(&binary, &raw).unwrap().is_some());
        }
    }
    for name in ["year-to-integer", "tinyint-to-integer"] {
        let mut integer = column(ValueKind::SignedInteger, "smallint");
        integer.transform = Some(name.into());
        assert!(!transformation_applied(&integer, &RawValue::Int(0)).unwrap());
        assert_eq!(
            encode_value(&integer, &RawValue::Int(0))
                .unwrap()
                .as_deref(),
            Some("0")
        );
    }
    let mut truth = column(ValueKind::Boolean, "boolean");
    truth.transform = Some("tinyint-to-boolean".into());
    for value in [-1, 2] {
        assert!(transformation_applied(&truth, &RawValue::Int(value)).unwrap());
        assert_eq!(
            encode_value(&truth, &RawValue::Int(value))
                .unwrap()
                .as_deref(),
            Some("true")
        );
    }
    for value in [0, 1] {
        assert!(!transformation_applied(&truth, &RawValue::Int(value)).unwrap());
    }
    let mut set = column(ValueKind::Set, "text[]");
    set.set_labels = vec!["a".into()];
    set.transform = Some("set-to-array".into());
    assert!(!transformation_applied(&set, &RawValue::UInt(1)).unwrap());
    assert_eq!(
        encode_value(&set, &RawValue::UInt(1)).unwrap().as_deref(),
        Some("{\"a\"}")
    );
    for name in [
        "zero-dates-to-null",
        "tinyint-to-boolean",
        "bits-to-boolean",
        "empty-string-to-null",
        "remove-null-characters",
        "right-trim",
        "byte-vector-to-bytea",
        "bytes-to-pg-bytea",
        "hex-to-bytea",
        "set-to-array",
        "year-to-integer",
        "tinyint-to-integer",
        "int-to-ip",
    ] {
        let mut null = column(ValueKind::Text, "text");
        null.transform = Some(name.into());
        assert!(!transformation_applied(&null, &RawValue::Null).unwrap());
    }
}

#[test]
fn explicit_utf8mb3_enforces_bmp_without_narrowing_generic_utf8_policies() {
    let mut text = column(ValueKind::Text, "text");
    text.charset = Some("utf8mb3".into());
    let bmp = RawValue::Bytes("Aé€\u{ffff}".as_bytes().to_vec());
    assert_eq!(
        encode_value(&text, &bmp).unwrap().as_deref(),
        Some("Aé€\u{ffff}")
    );
    assert_eq!(
        encode_row_limited(&row(vec![text.clone()]), &[bmp], 10).unwrap(),
        "Aé€\u{ffff}\n".as_bytes()
    );
    for value in ["\u{10000}", "😀"] {
        let raw = RawValue::Bytes(value.as_bytes().to_vec());
        assert!(encode_value(&text, &raw).is_err());
        assert!(
            encode_row_limited(&row(vec![text.clone()]), std::slice::from_ref(&raw), 5).is_err()
        );
        for policy in ["utf8", "utf8mb4"] {
            let mut generic = text.clone();
            generic.charset = Some(policy.into());
            assert_eq!(
                encode_value(&generic, &raw).unwrap().as_deref(),
                Some(value)
            );
        }
    }
    for malformed in [
        vec![0xed, 0xa0, 0x80],
        vec![0xc0, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
    ] {
        assert!(encode_value(&text, &RawValue::Bytes(malformed)).is_err());
    }
}
