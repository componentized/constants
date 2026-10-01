use anyhow::{Context, Result};
use componentized_constants::value::{Type, Value, WasmValue};
use componentized_constants::{Overrides, create_component};
use test_harness::{call, call_with};
use wasmtime::component::{ResourceType, Val};
use wit_parser::Resolve;

const ALL_WIT: &str = "tests/fixtures/all";
const ALL_OVERRIDES: &str = include_str!("fixtures/all/overrides.wave");

fn build(wit: &str, overrides: Option<&str>) -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let pkg = match wit.starts_with("package ") {
        true => resolve.push_str("test.wit", wit)?,
        false => resolve.push_path(wit)?.0,
    };
    let world = resolve.select_world(&[pkg], None)?;
    create_component(&resolve, world, overrides.map(Overrides::Wave))
}

fn build_with_value(wit: &str, overrides: &Value) -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let pkg = resolve.push_path(wit)?.0;
    let world = resolve.select_world(&[pkg], None)?;
    create_component(&resolve, world, Some(Overrides::Value(overrides)))
}

fn string(s: &str) -> Val {
    Val::String(s.into())
}

fn some(v: Val) -> Val {
    Val::Option(Some(Box::new(v)))
}

fn ok(v: Val) -> Val {
    Val::Result(Ok(Some(Box::new(v))))
}

fn err(v: Val) -> Val {
    Val::Result(Err(Some(Box::new(v))))
}

fn point(x: i32, y: i32) -> Val {
    Val::Record(vec![("x".into(), Val::S32(x)), ("y".into(), Val::S32(y))])
}

#[test]
fn it_returns_every_supported_type() -> Result<()> {
    let component = build(ALL_WIT, None)?;
    let iface = "example:constants/constants";
    let expected: Vec<(&str, &str, Val)> = vec![
        ("", "version", string("1.0.0")),
        (iface, "yes", Val::Bool(true)),
        (iface, "small", Val::S8(-128)),
        (iface, "medium", Val::U16(65535)),
        (iface, "answer", Val::U32(42)),
        (iface, "big", Val::U64(u64::MAX)),
        (iface, "negative", Val::S64(i64::MIN)),
        (iface, "pi", Val::Float32(1.5)),
        (iface, "e", Val::Float64(0.1)),
        (iface, "infinity", Val::Float64(f64::INFINITY)),
        (iface, "letter", Val::Char('🦀')),
        (iface, "greeting", string("hello, 🌎")),
        (iface, "empty", string("")),
        (iface, "http-port", Val::U16(8080)),
        (iface, "origin", point(1, -2)),
        (
            iface,
            "default-permissions",
            Val::Flags(vec!["read".into(), "write".into()]),
        ),
        (iface, "no-permissions", Val::Flags(vec![])),
        (
            iface,
            "primes",
            Val::List([2, 3, 5, 7, 11].into_iter().map(Val::U8).collect()),
        ),
        (iface, "nothing", Val::List(vec![])),
        (
            iface,
            "matrix",
            Val::List(vec![
                Val::List(vec![Val::Float64(1.0), Val::Float64(0.0)]),
                Val::List(vec![Val::Float64(0.0), Val::Float64(1.0)]),
                Val::List(vec![]),
            ]),
        ),
        (
            iface,
            "users",
            Val::List(vec![string("alice"), string("bob")]),
        ),
        (
            iface,
            "pair",
            Val::Tuple(vec![string("seconds"), Val::U64(60)]),
        ),
        (iface, "single", Val::Tuple(vec![Val::U8(7)])),
        (
            iface,
            "ports",
            Val::List(vec![
                Val::Tuple(vec![string("http"), Val::U16(80)]),
                Val::Tuple(vec![string("https"), Val::U16(443)]),
            ]),
        ),
        (
            iface,
            "servers",
            Val::List(vec![
                Val::Record(vec![
                    ("host".into(), string("a.example")),
                    ("port".into(), Val::U16(1)),
                    ("tags".into(), Val::List(vec![string("x"), string("y")])),
                ]),
                Val::Record(vec![
                    ("host".into(), string("b.example")),
                    ("port".into(), Val::U16(2)),
                    ("tags".into(), Val::List(vec![])),
                ]),
            ]),
        ),
        (
            iface,
            "rgb",
            Val::FixedLengthList(vec![Val::U8(255), Val::U8(128), Val::U8(0)]),
        ),
        (
            iface,
            "corners",
            Val::FixedLengthList(vec![point(0, 0), point(10, 10)]),
        ),
        (iface, "log-level", Val::Enum("info".into())),
        (
            iface,
            "levels",
            Val::List(vec![Val::Enum("debug".into()), Val::Enum("error".into())]),
        ),
        (iface, "no-shape", Val::Variant("none".into(), None)),
        (
            iface,
            "circle",
            Val::Variant("circle".into(), Some(Box::new(Val::Float32(1.5)))),
        ),
        (
            iface,
            "shapes",
            Val::List(vec![
                Val::Variant("square".into(), Some(Box::new(Val::U64(u64::MAX)))),
                Val::Variant("label".into(), Some(Box::new(string("hi")))),
                Val::Variant(
                    "polygon".into(),
                    Some(Box::new(Val::List(vec![point(0, 0), point(1, 1)]))),
                ),
                Val::Variant("none".into(), None),
            ]),
        ),
        (iface, "maybe", some(string("value"))),
        (iface, "outcome", ok(Val::U32(1))),
        (iface, "failure", err(string("boom"))),
        (iface, "bare-ok", ok(Val::U32(5))),
        (iface, "unit", Val::Result(Ok(None))),
        (iface, "unit-err", err(Val::Enum("error".into()))),
        (
            iface,
            "results",
            Val::List(vec![ok(point(1, 1)), err(string("no"))]),
        ),
        (iface, "maybe-result", some(ok(Val::U8(1)))),
        (iface, "maybe-not", Val::Option(None)),
        (iface, "bare", some(Val::U32(7))),
        (iface, "nested", some(Val::Option(None))),
        (
            iface,
            "sparse",
            Val::List(vec![some(Val::U8(1)), Val::Option(None), some(Val::U8(3))]),
        ),
        (
            iface,
            "profiles",
            Val::List(vec![
                Val::Record(vec![
                    ("name".into(), string("alice")),
                    ("nickname".into(), some(string("al"))),
                    ("age".into(), some(Val::U8(30))),
                    ("origin".into(), some(point(1, 2))),
                ]),
                Val::Record(vec![
                    ("name".into(), string("bob")),
                    ("nickname".into(), Val::Option(None)),
                    ("age".into(), Val::Option(None)),
                    ("origin".into(), Val::Option(None)),
                ]),
            ]),
        ),
    ];

    let calls: Vec<(&str, &str)> = expected.iter().map(|(i, n, _)| (*i, *n)).collect();
    let actual = call(&component, &["example:constants/types"], &calls)?;
    for ((_, name, expected), actual) in expected.iter().zip(actual) {
        assert_eq!(*expected, actual, "value for `{name}`");
    }
    Ok(())
}

#[test]
fn it_overrides_values_from_wit() -> Result<()> {
    let component = build(ALL_WIT, Some(ALL_OVERRIDES))?;
    let iface = "example:constants/constants";
    let actual = call(
        &component,
        &["example:constants/types"],
        &[
            ("", "version"),
            (iface, "answer"),
            (iface, "origin"),
            (iface, "greeting"),
        ],
    )?;
    assert_eq!(
        actual,
        vec![
            string("2.0.0"),
            Val::U32(7),
            point(5, 5),
            // not overridden
            string("hello, 🌎"),
        ]
    );
    Ok(())
}

#[test]
fn it_overrides_values_from_a_value() -> Result<()> {
    // the record types only need the fields being overridden
    let constants_ty = Type::record([("answer", Type::U32)]).context("constants type")?;
    let overrides_ty = Type::record([
        ("version", Type::STRING),
        ("constants", constants_ty.clone()),
    ])
    .context("overrides type")?;
    let overrides = Value::make_record(
        &overrides_ty,
        [
            ("version", Value::make_string("3.0.0".into())),
            (
                "constants",
                Value::make_record(&constants_ty, [("answer", Value::make_u32(9))])?,
            ),
        ],
    )?;

    let component = build_with_value(ALL_WIT, &overrides)?;
    let iface = "example:constants/constants";
    let actual = call(
        &component,
        &["example:constants/types"],
        &[("", "version"), (iface, "answer"), (iface, "greeting")],
    )?;
    assert_eq!(
        actual,
        vec![
            string("3.0.0"),
            Val::U32(9),
            // not overridden
            string("hello, 🌎"),
        ]
    );
    Ok(())
}

#[test]
fn it_rejects_a_value_of_the_wrong_type() -> Result<()> {
    let overrides_ty = Type::record([("version", Type::U32)]).context("overrides type")?;
    let overrides = Value::make_record(&overrides_ty, [("version", Value::make_u32(2))])?;
    let err = build_with_value(ALL_WIT, &overrides).expect_err("expected an error");
    assert!(
        format!("{err:#}").contains("invalid override for `version`"),
        "{err:#}"
    );
    Ok(())
}

#[test]
fn it_accepts_empty_overrides() -> Result<()> {
    for overrides in ["", "  // nothing\n", "{}", "{constants: {}}"] {
        let component = build(ALL_WIT, Some(overrides))?;
        let actual = call(&component, &["example:constants/types"], &[("", "version")])?;
        assert_eq!(actual, vec![string("1.0.0")], "{overrides:?}");
    }
    Ok(())
}

#[test]
fn it_reports_values_from_wit_by_location() {
    let err = format!(
        "{:#}",
        build(
            "package a:b;\nworld w {\n  /// @value 256\n  export n: func() -> u8;\n}",
            None
        )
        .unwrap_err()
    );
    assert!(
        err.contains("invalid `@value` for `n` at test.wit:4:10") && err.contains("expected u8"),
        "{err}"
    );

    let err = format!(
        "{:#}",
        build(
            "package a:b;\nworld w {\n  /// @value [\n  export n: func() -> u8;\n}",
            None
        )
        .unwrap_err()
    );
    assert!(
        err.contains("invalid `@value` for `n` at test.wit:4:10"),
        "{err}"
    );

    let err = format!(
        "{:#}",
        build(
            "package a:b;\ninterface i {\n  /// No value.\n  f: func() -> u8;\n}\nworld w { export i; }",
            None
        )
        .unwrap_err()
    );
    assert!(
        err.contains("missing value for `a:b/i#f` at test.wit:4:3"),
        "{err}"
    );
}

#[test]
fn it_imports_interfaces_defining_used_types() -> Result<()> {
    let component = build(ALL_WIT, None)?;
    let decoded = match wit_component::decode(&component)? {
        wit_component::DecodedWasm::Component(resolve, world) => (resolve, world),
        _ => unreachable!(),
    };
    let (resolve, world) = decoded;
    let imports: Vec<String> = resolve.worlds[world]
        .imports
        .keys()
        .map(|key| resolve.name_world_key(key))
        .collect();
    assert_eq!(imports, vec!["example:constants/types"]);
    Ok(())
}

#[test]
fn it_supports_inline_interfaces() -> Result<()> {
    let component = build(
        "package a:b;
        world w {
            export c: interface {
                variant v { x, y }
                d: func() -> u8;
                e: func() -> v;
            }
        }",
        Some("{c: {d: 1, e: y}}"),
    )?;
    let actual = call(&component, &[], &[("c", "d"), ("c", "e")])?;
    assert_eq!(actual, vec![Val::U8(1), Val::Variant("y".into(), None)]);
    Ok(())
}

fn build_err(wit: &str, overrides: &str) -> String {
    let err = build(wit, Some(overrides)).expect_err("expected an error");
    format!("{err:#}")
}

const UNSUPPORTED_WIT: &str = "package a:b;
    interface t {
        resource imported;
    }
    interface i {
        use t.{imported};
        resource res;
        type alias = future<string>;
        record r { f: list<alias> }
        variant v { a, b(r) }
        f: func() -> %type;
    }
    world w { export i; }";

#[test]
fn it_rejects_values_reaching_unsupported_types() {
    for (ty, value, unsupported) in [
        ("own<res>", "1", "`own<res>`"),
        ("option<own<imported>>", "some(1)", "`own<imported>`"),
        ("future<u8>", "1", "anonymous future"),
        ("list<stream<u8>>", "[1]", "anonymous stream"),
        ("error-context", "1", "`error-context`"),
        ("result<u8, alias>", "err(1)", "`alias` (future)"),
        ("r", "{f: [1]}", "`alias` (future)"),
        ("v", "b({f: [1]})", "`alias` (future)"),
        ("map<string, u8>", "[]", "anonymous map"),
        ("list<map<string, u8>>", "[[]]", "anonymous map"),
    ] {
        let wit = UNSUPPORTED_WIT.replace("%type", ty);
        let err = build_err(&wit, &format!("{{i: {{f: {value}}}}}"));
        let expected = format!("values of {unsupported} are not supported");
        assert!(
            err.contains(&expected),
            "{ty}: expected {expected:?} in {err:?}"
        );
    }
}

#[test]
fn it_allows_unsupported_types_in_branches_values_do_not_reach() -> Result<()> {
    for (ty, value, expected) in [
        ("option<own<res>>", "none", Val::Option(None)),
        ("option<own<imported>>", "none", Val::Option(None)),
        ("list<future<u8>>", "[]", Val::List(vec![])),
        (
            "result<u8, alias>",
            "ok(1)",
            Val::Result(Ok(Some(Box::new(Val::U8(1))))),
        ),
        (
            "r",
            "{f: []}",
            Val::Record(vec![("f".into(), Val::List(vec![]))]),
        ),
        ("v", "a", Val::Variant("a".into(), None)),
        ("option<error-context>", "none", Val::Option(None)),
        ("option<map<string, u8>>", "none", Val::Option(None)),
        (
            "tuple<u8, option<stream<u8>>>",
            "(1, none)",
            Val::Tuple(vec![Val::U8(1), Val::Option(None)]),
        ),
    ] {
        let wit = UNSUPPORTED_WIT.replace("%type", ty);
        let component =
            build(&wit, Some(&format!("{{i: {{f: {value}}}}}"))).with_context(|| ty.to_string())?;
        let actual = call_with(
            &component,
            |linker| {
                linker.instance("a:b/t")?.resource(
                    "imported",
                    ResourceType::host::<()>(),
                    |_, _| Ok(()),
                )?;
                Ok(())
            },
            &[("a:b/i", "f")],
        )
        .with_context(|| ty.to_string())?;
        assert_eq!(actual, vec![expected], "{ty}");
    }
    Ok(())
}

#[test]
fn it_rejects_non_constant_functions() {
    let err = build_err(
        "package a:b; world w { export f: func(x: u8) -> u8; }",
        "{f: 1}",
    );
    assert!(err.contains("must not accept parameters"), "{err}");

    let err = build_err("package a:b; world w { export f: func(); }", "{f: 1}");
    assert!(err.contains("must return a value"), "{err}");
}

#[test]
fn it_rejects_invalid_values() {
    let wit = "package a:b;
        world w {
            record p { x: u8 }
            flags f { a }
            export n: func() -> u8;
            export s: func() -> string;
            export r: func() -> p;
            export g: func() -> f;
            export l: func() -> list<u8, 2>;
            enum e { a }
            variant v { a, b(u8) }
            export e: func() -> e;
            export v: func() -> v;
            export o: func() -> option<option<u8>>;
            export x: func() -> result<u8, string>;
            export y: func() -> result<option<u8>>;
        }";
    let valid = r#"n: 1, s: "", r: {x: 1}, g: {a}, l: [1, 2], e: a, v: b(1), o: some(1), x: ok(1), y: ok(none)"#;
    build(wit, Some(&format!("{{{valid}}}"))).expect("valid values");

    for (values, expected) in [
        (r#"{n: 1}"#, "missing value for `s`"),
        (
            &format!("{{{valid}, extra: 1}}"),
            "unexpected field `extra`",
        ),
        (&valid.replace("n: 1", "n: 1, n: 2"), "duplicate field"),
        (&valid.replace("n: 1", "n: 256"), "expected u8"),
        (&valid.replace("n: 1", "n: -1"), "expected u8"),
        (&valid.replace(r#"s: """#, "s: 1"), "invalid value type"),
        (
            &valid.replace("{x: 1}", "{x: 1, y: 2}"),
            "unexpected field `y`",
        ),
        (&valid.replace("{a}", "{b}"), "unknown flag `b`"),
        (&valid.replace("[1, 2]", "[1]"), "expected 2 elements"),
        (&valid.replace("e: a", "e: z"), "unknown enum case `z`"),
        (
            &valid.replace("v: b(1)", "v: z"),
            "unknown variant case `z`",
        ),
        (
            &valid.replace("v: b(1)", "v: b"),
            "case `b` requires a payload",
        ),
        (
            &valid.replace("v: b(1)", "v: a(1)"),
            "case `a` has no payload",
        ),
        (&valid.replace("v: b(1)", "v: b(256)"), "expected u8"),
        (
            &valid.replace("o: some(1)", "o: 1"),
            "options of options or results must be written",
        ),
        (
            &valid.replace("o: some(1)", "o: some(some(-1))"),
            "expected u8",
        ),
        (&valid.replace("r: {x: 1}", "r: {}"), "missing field `x`"),
        (&valid.replace("x: ok(1)", "x: ok(256)"), "expected u8"),
        (&valid.replace("x: ok(1)", "x: 256"), "expected u8"),
        (
            &valid.replace("x: ok(1)", "x: err"),
            "case `err` requires a payload",
        ),
        (
            &valid.replace("x: ok(1)", "x: err(1)"),
            "invalid value type",
        ),
        (
            &valid.replace("y: ok(none)", "y: none"),
            "results must be written as `ok(...)` or `err(...)`",
        ),
        (
            &valid.replace("y: ok(none)", "y: err(1)"),
            "case `err` has no payload",
        ),
    ] {
        let values = if values.starts_with('{') {
            values.to_string()
        } else {
            format!("{{{values}}}")
        };
        let err = build_err(wit, &values);
        assert!(
            err.contains(expected),
            "{values}: expected {expected:?} in {err:?}"
        );
    }
}
