use anyhow::{Context, Result};
use componentized_constants::value::{Type, Value, WasmValue};
use componentized_constants::{Overrides, create_component};
use test_harness::{call, call_with, read_streams, read_streams_until, read_streams_with};
use wasmtime::component::{ComponentType, Lift, ResourceType, Val};
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

const STREAMS_WIT: &str = "package a:b;
    interface i {
        type bytes = stream<u8>;
        /// @value [1, 2, 3, 4, 5]
        numbers: async func() -> bytes;
        /// @value [\"a\", \"bc\", \"\", \"🦀\"]
        words: async func() -> stream<string>;
        /// @value [{x: 1, y: -1}, {x: -2, y: 2}]
        points: async func() -> stream<point>;
        /// @value []
        nothing: async func() -> stream<u8>;
        /// @value []
        closed: func() -> stream<u8>;
        /// @value 42
        answer: async func() -> u32;
        record point { x: s32, y: s32 }
    }
    world w {
        export i;
        /// @value [true, false]
        export switches: async func() -> stream<bool>;
    }";

#[derive(ComponentType, Lift, Debug, PartialEq, Clone)]
#[component(record)]
struct Point {
    x: i32,
    y: i32,
}

#[test]
fn it_returns_streams() -> Result<()> {
    let component = build(STREAMS_WIT, None)?;
    let iface = "a:b/i";
    // reads taking every item, and reads taking fewer items than are written
    for per_read in [100, 2, 1] {
        let context = || format!("{per_read} items per read");
        let numbers = read_streams::<u8>(&component, (iface, "numbers"), per_read, 3)
            .with_context(context)?;
        assert_eq!(numbers, vec![vec![1, 2, 3, 4, 5]; 6], "{}", context());
        let words = read_streams::<String>(&component, (iface, "words"), per_read, 2)
            .with_context(context)?;
        let expected: Vec<String> = ["a", "bc", "", "🦀"].map(String::from).into();
        assert_eq!(words, vec![expected; 4], "{}", context());
        let points = read_streams::<Point>(&component, (iface, "points"), per_read, 1)
            .with_context(context)?;
        let expected = vec![Point { x: 1, y: -1 }, Point { x: -2, y: 2 }];
        assert_eq!(points, vec![expected; 2], "{}", context());
        let switches = read_streams::<bool>(&component, ("", "switches"), per_read, 1)
            .with_context(context)?;
        assert_eq!(switches, vec![vec![true, false]; 2], "{}", context());
    }
    for name in ["nothing", "closed"] {
        let empty = read_streams::<u8>(&component, (iface, name), 1, 2)?;
        assert_eq!(empty, vec![Vec::<u8>::new(); 4], "{name}");
    }
    Ok(())
}

#[test]
fn it_stops_writing_when_the_reader_closes_the_stream() -> Result<()> {
    let component = build(STREAMS_WIT, None)?;
    // the second round of calls shows the closed streams' tasks ended cleanly
    let numbers = read_streams_until::<u8>(&component, ("a:b/i", "numbers"), 1, 2, 2)?;
    assert_eq!(numbers, vec![vec![1, 2]; 4]);
    Ok(())
}

#[test]
fn it_grows_memory_for_the_state_of_each_stream() -> Result<()> {
    // the data starts at 8 and ends just short of the first page, so the state
    // of a second stream needs another page
    let items = vec!["7"; 65536 - 8 - 8].join(",");
    let component = build(
        "package a:b; world w { export f: async func() -> stream<u8>; }",
        Some(&format!("{{f: [{items}]}}")),
    )?;
    let streams = read_streams::<u8>(&component, ("", "f"), 65536, 3)?;
    assert_eq!(streams, vec![vec![7; 65536 - 16]; 6]);
    Ok(())
}

#[test]
fn it_returns_values_from_async_functions() -> Result<()> {
    let component = build(STREAMS_WIT, None)?;
    let actual = call(&component, &[], &[("a:b/i", "answer")])?;
    assert_eq!(actual, vec![Val::U32(42)]);
    Ok(())
}

#[test]
fn it_overrides_streams() -> Result<()> {
    let component = build(STREAMS_WIT, Some("{i: {numbers: [9], closed: []}}"))?;
    let numbers = read_streams::<u8>(&component, ("a:b/i", "numbers"), 1, 1)?;
    assert_eq!(numbers, vec![vec![9]; 2]);
    Ok(())
}

#[test]
fn it_rejects_streams_with_items_from_sync_functions() {
    let err = build_err(
        "package a:b; world w { export f: func() -> stream<u8>; }",
        "{f: [1]}",
    );
    assert!(
        err.contains("a stream with items must be returned by an async function"),
        "{err}"
    );
}

#[test]
fn it_rejects_invalid_stream_items() {
    let wit = "package a:b; world w { export f: async func() -> stream<u8>; }";
    for (value, expected) in [
        ("[256]", "expected u8"),
        ("1", "invalid value type"),
        ("[\"a\"]", "invalid value type"),
    ] {
        let err = build_err(wit, &format!("{{f: {value}}}"));
        assert!(
            err.contains(expected),
            "{value}: expected {expected:?} in {err:?}"
        );
    }

    // a stream without a type has no items to write
    let err = build_err(
        "package a:b; world w { export f: async func() -> stream; }",
        "{f: []}",
    );
    assert!(err.contains("are not supported"), "{err}");
}

const GENERATORS_WIT: &str = "package a:b;
    world w {
        /// @value [0]
        /// @expression |n| n + 1
        export incrementor: async func() -> stream<u8>;
        /// @value [false]
        /// @expression |value| !value
        export flip-flop: async func() -> stream<bool>;
        /// @value [1, 1]
        /// @expression |a, b| a + b
        export fibonacci: async func() -> stream<u32>;
        /// @value [1, 1]
        /// @expression |a, b| a + b
        export long-fibonacci: async func() -> stream<u64>;
        /// @value [0]
        /// @expression |n| n - 1
        export countdown: async func() -> stream<s8>;
        /// @value [1]
        /// @expression |n| n * 2
        export doubling: async func() -> stream<u64>;
        /// @value [9223372036854775805]
        /// @expression |n| n + 1
        export near-max: async func() -> stream<s64>;
        /// @value [1.0]
        /// @expression |x| x * 10.0
        export powers: async func() -> stream<f64>;
        /// @value [1]
        /// @expression |x| x * 10
        export small-powers: async func() -> stream<f32>;
        /// @value [3]
        /// @expression |n| 12 / (n - 1)
        export divide: async func() -> stream<s32>;
        /// @value [0]
        /// @expression |n| if n == 9 { 0 } else { n + 1 }
        export digits: async func() -> stream<u8>;
        /// @value [1]
        /// @expression |n| n << 1
        export shift: async func() -> stream<u32>;
        /// @value [1]
        /// @expression |n| n << 1
        export signed-shift: async func() -> stream<s8>;
        /// @value []
        /// @expression || 7
        export sevens: async func() -> stream<u8>;
        /// @value [1, 2, 3]
        /// @expression |a, b| a + b
        export window: async func() -> stream<u16>;
        /// @value [-5]
        /// @expression |n| if n < 0 && n % 2 != 0 { -n } else { n - 3 }
        export bounce: async func() -> stream<s16>;
    }";

fn generated<T: wasmtime::component::Lift + PartialEq + std::fmt::Debug + Send + Sync + 'static>(
    component: &[u8],
    name: &str,
    limit: usize,
) -> Result<Vec<T>> {
    let mut streams = vec![];
    // reads taking every item, and reads taking fewer items than are written
    for per_read in [1000, 7, 1] {
        let read = read_streams_until::<T>(component, ("", name), per_read, 2, limit)
            .with_context(|| format!("{name}, {per_read} items per read"))?;
        streams.extend(read);
    }
    let first = streams.remove(0);
    for stream in streams {
        assert_eq!(stream, first, "{name}: streams differ");
    }
    Ok(first)
}

/// The sequence from `seed`, until `next` can't produce an item.
fn sequence<T: Copy>(seed: &[T], next: impl Fn(&[T]) -> Option<T>) -> Vec<T> {
    let mut items = seed.to_vec();
    while let Some(item) = next(&items) {
        items.push(item);
    }
    items
}

#[test]
fn it_generates_streams_until_an_item_cannot_be_represented() -> Result<()> {
    let component = build(GENERATORS_WIT, None)?;
    let all = usize::MAX;
    assert_eq!(
        generated::<u8>(&component, "incrementor", all)?,
        (0..=255).collect::<Vec<u8>>()
    );
    assert_eq!(
        generated::<u32>(&component, "fibonacci", all)?,
        sequence(&[1u32, 1], |s| s[s.len() - 2].checked_add(s[s.len() - 1]))
    );
    assert_eq!(
        generated::<u64>(&component, "long-fibonacci", all)?,
        sequence(&[1u64, 1], |s| s[s.len() - 2].checked_add(s[s.len() - 1]))
    );
    assert_eq!(
        generated::<i8>(&component, "countdown", all)?,
        (-128..=0).rev().collect::<Vec<i8>>()
    );
    assert_eq!(
        generated::<u64>(&component, "doubling", all)?,
        (0..64).map(|n| 1u64 << n).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i64>(&component, "near-max", all)?,
        vec![i64::MAX - 2, i64::MAX - 1, i64::MAX]
    );
    assert_eq!(
        generated::<f64>(&component, "powers", all)?,
        sequence(&[1.0f64], |s| Some(s[s.len() - 1] * 10.0)
            .filter(|x| x.is_finite()))
    );
    assert_eq!(
        generated::<f32>(&component, "small-powers", all)?,
        sequence(&[1.0f32], |s| Some(s[s.len() - 1] * 10.0)
            .filter(|x| x.is_finite()))
    );
    assert_eq!(
        generated::<i32>(&component, "divide", all)?,
        vec![3, 6, 2, 12, 1]
    );
    Ok(())
}

#[test]
fn it_generates_unbounded_streams() -> Result<()> {
    let component = build(GENERATORS_WIT, None)?;
    assert_eq!(
        generated::<bool>(&component, "flip-flop", 5)?,
        vec![false, true, false, true, false]
    );
    assert_eq!(
        generated::<u8>(&component, "digits", 12)?,
        vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1]
    );
    // bits shifted past the type's width are dropped, as in Rust
    let mut shifts: Vec<u32> = (0..32).map(|n| 1 << n).collect();
    shifts.extend([0, 0]);
    assert_eq!(generated::<u32>(&component, "shift", 34)?, shifts);
    assert_eq!(
        generated::<i8>(&component, "signed-shift", 10)?,
        vec![1, 2, 4, 8, 16, 32, 64, -128, 0, 0]
    );
    assert_eq!(generated::<u8>(&component, "sevens", 3)?, vec![7, 7, 7]);
    assert_eq!(
        generated::<u16>(&component, "window", 7)?,
        vec![1, 2, 3, 5, 8, 13, 21]
    );
    assert_eq!(
        generated::<i16>(&component, "bounce", 6)?,
        vec![-5, 5, 2, -1, 1, -2]
    );
    // longer than a batch of generated items
    assert_eq!(
        generated::<bool>(&component, "flip-flop", 1000)?,
        (0..1000).map(|n| n % 2 == 1).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn it_overrides_the_items_generated_streams_start_from() -> Result<()> {
    let component = build(
        GENERATORS_WIT,
        Some("{incrementor: [250], fibonacci: [5, 8, 13]}"),
    )?;
    assert_eq!(
        generated::<u8>(&component, "incrementor", usize::MAX)?,
        (250..=255).collect::<Vec<u8>>()
    );
    assert_eq!(
        generated::<u32>(&component, "fibonacci", 5)?,
        vec![5, 8, 13, 21, 34]
    );
    Ok(())
}

#[test]
fn it_rejects_invalid_expressions() {
    let wit = |ty: &str, value: &str, expression: &str| {
        format!(
            "package a:b; world w {{
                /// @value {value}
                /// @expression {expression}
                export f: async func() -> {ty};
            }}"
        )
    };
    for (wit, expected) in [
        (wit("u8", "1", "|n| n"), "only functions returning a stream"),
        (
            wit("stream<string>", "[\"a\"]", "|s| s"),
            "only streams of integers, floats or bool",
        ),
        (wit("stream<u8>", "[1]", "|a, b| a + b"), "list at least 2"),
        (wit("stream<u8>", "[1]", "|n| m"), "unknown name `m`"),
        (wit("stream<u8>", "[1]", "|n| n + true"), "mismatched types"),
        (
            wit("stream<u8>", "[1]", "|n| n + 256"),
            "literal 256 is out of range for u8",
        ),
        (wit("stream<u8>", "[1]", "|n| -n"), "`-` can't negate u8"),
        (
            wit("stream<u8>", "[1]", "|n| n + 1.5"),
            "expected u8, found float",
        ),
        (
            wit("stream<f64>", "[1.0]", "|x| x % 2.0"),
            "isn't supported for f64",
        ),
        (
            wit("stream<u8>", "[1]", "|n| n < 2 < 3"),
            "can't be chained",
        ),
        (
            wit("stream<u8>", "[1]", "|n| if n > 1 { 1 }"),
            "expected `else`",
        ),
        (
            wit("stream<u8>", "[1]", "|n| n > 1"),
            "expected the closure to return u8",
        ),
        (
            wit("stream<u8>", "[1]", "|n| n + 1u8"),
            "literal suffixes aren't supported",
        ),
        (wit("stream<u8>", "[1]", "n + 1"), "expected `|`"),
        (
            wit("stream<u8>", "[1]", "|n| n +"),
            "expected an expression at the end",
        ),
        (
            wit("stream<u8>", "[1]", "|n, n| n"),
            "duplicate parameter `n`",
        ),
    ] {
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }

    // positions are the line and column within the expression
    let err = format!(
        "{:#}",
        build(
            "package a:b; world w {
                /// @value [1]
                /// @expression |n|
                ///   n +   $
                export f: async func() -> stream<u8>;
            }",
            None
        )
        .expect_err("position")
    );
    assert!(err.contains("unexpected `$` at 2:9"), "{err}");

    // generated streams are unbounded, so need an async function
    let err = format!(
        "{:#}",
        build(
            "package a:b; world w {
                /// @value []
                /// @expression || 1
                export f: func() -> stream<u8>;
            }",
            None
        )
        .expect_err("sync")
    );
    assert!(
        err.contains("must be returned by an async function"),
        "{err}"
    );
}

const ARGUMENTS_WIT: &str = "package a:b;
    interface i {
        /// @expression |a, b| a + b
        fibonacci: async func(a: u32, b: u32) -> stream<u32>;
    }
    world w {
        export i;
        /// @expression |n| n + 1
        export incrementor: async func(n: u8) -> stream<u8>;
        /// @expression |value| !value
        export flip-flop: async func(value: bool) -> stream<bool>;
        /// @expression |x| x / 2.0
        export halving: async func(x: f64) -> stream<f64>;
    }";

#[test]
fn it_generates_streams_from_arguments() -> Result<()> {
    let component = build(ARGUMENTS_WIT, None)?;
    let all = usize::MAX;
    for per_read in [1000, 7, 1] {
        let fibonacci = read_streams_with::<_, u32>(
            &component,
            ("a:b/i", "fibonacci"),
            (1u32, 1u32),
            per_read,
            2,
            all,
        )?;
        let expected = sequence(&[1u32, 1], |s| s[s.len() - 2].checked_add(s[s.len() - 1]));
        assert_eq!(fibonacci, vec![expected; 4], "{per_read} items per read");
    }
    let fibonacci =
        read_streams_with::<_, u32>(&component, ("a:b/i", "fibonacci"), (5u32, 8u32), 1, 1, 4)?;
    assert_eq!(fibonacci, vec![vec![5, 8, 13, 21]; 2]);
    let incrementor =
        read_streams_with::<_, u8>(&component, ("", "incrementor"), (250u8,), 3, 1, all)?;
    assert_eq!(incrementor, vec![(250..=255).collect::<Vec<u8>>(); 2]);
    let flip_flop = read_streams_with::<_, bool>(&component, ("", "flip-flop"), (true,), 2, 1, 3)?;
    assert_eq!(flip_flop, vec![vec![true, false, true]; 2]);
    let halving = read_streams_with::<_, f64>(&component, ("", "halving"), (8.0,), 2, 1, 4)?;
    assert_eq!(halving, vec![vec![8.0, 4.0, 2.0, 1.0]; 2]);
    Ok(())
}

#[test]
fn it_rejects_overrides_for_streams_generated_from_arguments() {
    for overrides in ["{incrementor: [1]}", "{i: {fibonacci: [1, 1]}}"] {
        let err = format!(
            "{:#}",
            build(ARGUMENTS_WIT, Some(overrides)).expect_err(overrides)
        );
        assert!(
            err.contains("can't be overridden, its stream is generated from its arguments"),
            "{overrides}: {err}"
        );
    }
}

#[test]
fn it_rejects_invalid_streams_generated_from_arguments() {
    let wit =
        |docs: &str, func: &str| format!("package a:b; world w {{ {docs} export f: {func}; }}");
    for (wit, expected) in [
        (
            wit(
                "/// @expression |a, b| a + b\n",
                "async func(a: u32) -> stream<u32>",
            ),
            "the expression takes 2 parameters, the function takes 1 parameter",
        ),
        (
            wit(
                "/// @expression |a| a\n",
                "async func(a: u32, b: u32) -> stream<u32>",
            ),
            "the expression takes 1 parameter, the function takes 2 parameters",
        ),
        (
            wit(
                "/// @expression |a| a\n",
                "async func(a: u8) -> stream<u32>",
            ),
            "parameter `a` must have the stream's item type, `u32`",
        ),
        (
            wit("/// @expression |a| a\n", "func(a: u32) -> stream<u32>"),
            "must be returned by an async function",
        ),
        (
            wit(
                "/// @value [1]\n/// @expression |a| a\n",
                "async func(a: u32) -> stream<u32>",
            ),
            "its stream starts from its arguments, remove the tag",
        ),
        (
            wit("/// @value [1]\n", "async func(a: u32) -> stream<u32>"),
            "must not accept parameters, unless it generates a stream",
        ),
        (
            wit("/// @expression |a| a\n", "async func(a: u32) -> u32"),
            "only functions returning a stream can generate items",
        ),
    ] {
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }
}

#[test]
fn it_rejects_duplicate_tags() {
    for docs in [
        "/// @value 1\n/// @value 2\n",
        "/// @value [1]\n/// @expression |n| n + 1\n/// @expression |n| n + 2\n",
    ] {
        let wit = format!(
            "package a:b; interface i {{ {docs} f: async func() -> stream<u8>; }} world w {{ export i; }}"
        );
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(
            err.contains("interface `i`: function `f`: duplicate `@"),
            "{docs}: {err}"
        );
    }
}

#[test]
fn it_generates_xorshift_pseudorandom_numbers() -> Result<()> {
    // xorshift64, x ^= x << 13; x ^= x >> 7; x ^= x << 17, written out without `let`
    let component = build(
        "package a:b; world w {
            /// @expression |x| ((x ^ x << 13) ^ (x ^ x << 13) >> 7) ^ ((x ^ x << 13) ^ (x ^ x << 13) >> 7) << 17
            export pseudorandom: async func(seed: u64) -> stream<u64>;
        }",
        None,
    )?;
    let xorshift = |mut x: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for seed in [1u64, 42, u64::MAX] {
        let mut expected = vec![seed];
        while expected.len() < 1000 {
            expected.push(xorshift(*expected.last().unwrap()));
        }
        let actual =
            read_streams_with::<_, u64>(&component, ("", "pseudorandom"), (seed,), 100, 1, 1000)?;
        assert_eq!(actual, vec![expected; 2], "seed {seed}");
    }
    Ok(())
}

const SEEDED_WIT: &str = "package a:b;
    world w {
        /// SplitMix64, from the seed and the position
        /// @expression || {
        ///     let z = $init.0.wrapping_add($i.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15));
        ///     let z = (z ^ z >> 30).wrapping_mul(0xBF58476D1CE4E5B9);
        ///     let z = (z ^ z >> 27).wrapping_mul(0x94D049BB133111EB);
        ///     z ^ z >> 31
        /// }
        export splitmix: async func(seed: u64) -> stream<u64>;
        /// SplitMix64, with wrapping operators
        /// @expression || {
        ///     let z = $init.0 +% ($i +% 1) *% 0x9E3779B97F4A7C15;
        ///     let z = (z ^ z >> 30) *% 0xBF58476D1CE4E5B9;
        ///     let z = (z ^ z >> 27) *% 0x94D049BB133111EB;
        ///     z ^ z >> 31
        /// }
        export splitmix-operators: async func(seed: u64) -> stream<u64>;
        /// @expression || ($init.0 as u64 * $i) as u8
        export truncated: async func(seed: u32) -> stream<u8>;
        /// @value [1]
        /// @expression |n| n + $init.0 + $init.1 as u32
        export stepped: async func(step: u32, extra: u8) -> stream<u32>;
        /// @expression || $init.0 && $i % 2 == 0
        export gated: async func(open: bool) -> stream<bool>;
    }";

/// SplitMix64 as Rust, from a seed and a position.
fn splitmix(seed: u64, i: u64) -> u64 {
    let z = seed.wrapping_add(i.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15));
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

#[test]
fn it_generates_streams_from_seed_and_position() -> Result<()> {
    let component = build(SEEDED_WIT, None)?;
    for seed in [0u64, 42, u64::MAX] {
        let actual =
            read_streams_with::<_, u64>(&component, ("", "splitmix"), (seed,), 100, 2, 1000)?;
        let expected: Vec<u64> = (0..1000).map(|i| splitmix(seed, i)).collect();
        assert_eq!(actual, vec![expected.clone(); 4], "seed {seed}");
        let actual = read_streams_with::<_, u64>(
            &component,
            ("", "splitmix-operators"),
            (seed,),
            100,
            1,
            1000,
        )?;
        assert_eq!(actual, vec![expected; 2], "seed {seed}");
    }
    let truncated = read_streams_with::<_, u8>(&component, ("", "truncated"), (7u32,), 9, 1, 300)?;
    let expected: Vec<u8> = (0..300u64).map(|i| (7 * i) as u8).collect();
    assert_eq!(truncated, vec![expected; 2]);
    // the listed items come first, and the arguments aren't items
    let stepped = read_streams_with::<_, u32>(&component, ("", "stepped"), (10u32, 5u8), 3, 1, 4)?;
    assert_eq!(stepped, vec![vec![1, 16, 31, 46]; 2]);
    let gated = read_streams_with::<_, bool>(&component, ("", "gated"), (true,), 3, 1, 4)?;
    assert_eq!(gated, vec![vec![true, false, true, false]; 2]);
    Ok(())
}

#[test]
fn it_reads_the_position() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @value [10, 20]
            /// @expression |_, b| b + $i
            export f: async func() -> stream<u64>;
        }",
        None,
    )?;
    // the first item generated is at position 2
    assert_eq!(
        generated::<u64>(&component, "f", 5)?,
        vec![10, 20, 22, 25, 29]
    );
    Ok(())
}

const CASTS_WIT: &str = "package a:b;
    world w {
        /// @value []
        /// @expression || ($i as i64 - 3) as u8
        export negative-to-unsigned: async func() -> stream<u8>;
        /// @value []
        /// @expression || ($i as f64 * 100.0 - 150.0) as i8
        export saturating: async func() -> stream<s8>;
        /// @value []
        /// @expression || ($i as f64 * 1.0e10) as u32
        export saturating-wide: async func() -> stream<u32>;
        /// @value []
        /// @expression || ($i % 2 == 0) as u8
        export from-bool: async func() -> stream<u8>;
        /// @value []
        /// @expression || ($i as i16 * 1000) as i8
        export truncating-signed: async func() -> stream<s8>;
        /// @value []
        /// @expression || $i as f32 / 4.0
        export to-float: async func() -> stream<f32>;
        /// @value []
        /// @expression || ($i as u8).wrapping_add(250)
        export wrapping-add: async func() -> stream<u8>;
        /// @value []
        /// @expression || ($i as i8).wrapping_mul(100)
        export wrapping-mul: async func() -> stream<s8>;
        /// @value []
        /// @expression || (0 as u16).wrapping_sub($i as u16)
        export wrapping-sub: async func() -> stream<u16>;
        /// @value []
        /// @expression || (0b1000_0101 as u8).rotate_left($i as u32)
        export rotate-u8: async func() -> stream<u8>;
        /// @value []
        /// @expression || (-30000 as i16).rotate_right($i as u32)
        export rotate-s16: async func() -> stream<s16>;
        /// @value []
        /// @expression || { let x: u32 = 0x8000_0001; x.rotate_left($i as u32) }
        export rotate-u32: async func() -> stream<u32>;
        /// @value []
        /// @expression || (-2 as i64).rotate_right($i as u32)
        export rotate-s64: async func() -> stream<s64>;
        /// @value []
        /// @expression || { let a: u8 = 3; let b = a * 2; let a = b + a; { let _ = $i; a } }
        export lets: async func() -> stream<u8>;
    }";

#[test]
fn it_casts_and_wraps_as_rust() -> Result<()> {
    let c = build(CASTS_WIT, None)?;
    let n = 70u64;
    assert_eq!(
        generated::<u8>(&c, "negative-to-unsigned", 5)?,
        (0..5).map(|i: i64| (i - 3) as u8).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i8>(&c, "saturating", 4)?,
        (0..4)
            .map(|i| (i as f64 * 100.0 - 150.0) as i8)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u32>(&c, "saturating-wide", 3)?,
        (0..3)
            .map(|i| (i as f64 * 1.0e10) as u32)
            .collect::<Vec<_>>()
    );
    assert_eq!(generated::<u8>(&c, "from-bool", 4)?, vec![1, 0, 1, 0]);
    assert_eq!(
        generated::<i8>(&c, "truncating-signed", 30)?,
        (0..30).map(|i: i16| (i * 1000) as i8).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<f32>(&c, "to-float", 5)?,
        (0..5).map(|i| i as f32 / 4.0).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u8>(&c, "wrapping-add", n as usize)?,
        (0..n)
            .map(|i| (i as u8).wrapping_add(250))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i8>(&c, "wrapping-mul", n as usize)?,
        (0..n)
            .map(|i| (i as i8).wrapping_mul(100))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u16>(&c, "wrapping-sub", n as usize)?,
        (0..n)
            .map(|i| 0u16.wrapping_sub(i as u16))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u8>(&c, "rotate-u8", n as usize)?,
        (0..n)
            .map(|i| 0b1000_0101u8.rotate_left(i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i16>(&c, "rotate-s16", n as usize)?,
        (0..n)
            .map(|i| (-30000i16).rotate_right(i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u32>(&c, "rotate-u32", n as usize)?,
        (0..n)
            .map(|i| 0x8000_0001u32.rotate_left(i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i64>(&c, "rotate-s64", n as usize)?,
        (0..n)
            .map(|i| (-2i64).rotate_right(i as u32))
            .collect::<Vec<_>>()
    );
    assert_eq!(generated::<u8>(&c, "lets", 2)?, vec![9, 9]);
    Ok(())
}

#[test]
fn it_rejects_invalid_seeds_casts_methods_and_lets() {
    let wit = |expression: &str, func: &str| {
        format!(
            "package a:b; world w {{
                /// @value []
                /// @expression {expression}
                export f: {func};
            }}"
        )
    };
    for (wit, expected) in [
        (
            wit("|| $init.0", "async func() -> stream<u8>"),
            "`$init` holds the function's arguments, but the function takes none",
        ),
        (
            wit("|| $init.1", "async func(a: u8) -> stream<u8>"),
            "`$init.1` doesn't exist, `$init` has 1 fields",
        ),
        (
            wit("|| $init", "async func(a: u8) -> stream<u8>"),
            "expected `.` and a field, `$init` is a tuple",
        ),
        (
            wit("|| $init.0 as u8", "async func(a: string) -> stream<u8>"),
            "parameter `a` can't be read with `$init`",
        ),
        (
            wit("|| $x", "async func() -> stream<u8>"),
            "unknown variable `$x`",
        ),
        (
            wit("|| $i", "async func() -> stream<u8>"),
            "expected the closure to return u8, found u64",
        ),
        (
            wit("|| $init.0 * $i", "async func(seed: u32) -> stream<u8>"),
            "mismatched types for `*`, u32 and u64",
        ),
        (
            wit("|| $i as bool", "async func() -> stream<bool>"),
            "can't cast to bool",
        ),
        (
            wit("|| 1.wrapping_add(2)", "async func() -> stream<u8>"),
            "can't call `wrapping_add` on a literal",
        ),
        (
            wit(
                "|| ($i as u8).saturating_add(2)",
                "async func() -> stream<u8>",
            ),
            "unknown method `saturating_add`",
        ),
        (
            wit("|| { let x = 1; x }", "async func() -> stream<u8>"),
            "the type of a `let` bound to a literal must be given",
        ),
        (
            wit("|| { let x: u8 = 1 x }", "async func() -> stream<u8>"),
            "expected `;`",
        ),
        (
            wit(
                "|| ($i as f64).rotate_left(1)",
                "async func() -> stream<f64>",
            ),
            "`rotate_left` isn't supported for f64",
        ),
        (
            wit("|| $i as u9", "async func() -> stream<u8>"),
            "expected a type",
        ),
        (
            wit("|| { let as: u8 = 1; as }", "async func() -> stream<u8>"),
            "expected a name to bind",
        ),
    ] {
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }

    // overrides are rejected, even with listed items
    let err = format!(
        "{:#}",
        build(SEEDED_WIT, Some("{stepped: [5]}")).expect_err("overrides")
    );
    assert!(err.contains("can't be overridden"), "{err}");
}

/// Wrapping operators on every integer type, against Rust's `wrapping_*`
/// methods: every pair of operands for 8-bit types, and pseudorandom pairs
/// for wider types, both derived from the position.
#[test]
fn it_wraps_with_wrapping_operators_as_rust() -> Result<()> {
    const OPS: [(&str, &str); 8] = [
        ("add", "+%"),
        ("sub", "-%"),
        ("mul", "*%"),
        ("div", "/%"),
        ("rem", "%%"),
        ("shl", "<<%"),
        ("shr", ">>%"),
        ("neg", "-%"),
    ];
    let types = [
        ("u8", "u8", true),
        ("s8", "i8", true),
        ("u16", "u16", false),
        ("s16", "i16", false),
        ("u32", "u32", false),
        ("s32", "i32", false),
        ("u64", "u64", false),
        ("s64", "i64", false),
    ];
    let mut wit = String::from("package a:b; world w {\n");
    for (wit_ty, rust_ty, exhaustive) in types {
        let (a, b) = match exhaustive {
            true => (
                format!("($i >> 8) as u8 as {rust_ty}"),
                format!("$i as u8 as {rust_ty}"),
            ),
            false => (
                format!("($i *% 0x9E3779B97F4A7C15 ^ $i >> 7) as {rust_ty}"),
                format!("($i *% 0xD1B54A32D192ED03 ^ $i >> 3) as {rust_ty}"),
            ),
        };
        for (name, op) in OPS {
            let value = match name {
                "neg" => "-%a".to_string(),
                // a zero divisor still ends the stream
                "div" | "rem" => format!("a {op} if b == 0 {{ 1 }} else {{ b }}"),
                _ => format!("a {op} b"),
            };
            wit.push_str(&format!(
                "/// @value []\n/// @expression || {{ let a = {a}; let b = {b}; {value} }}\n\
                 export {name}-{wit_ty}: async func() -> stream<{wit_ty}>;\n"
            ));
        }
    }
    wit.push('}');
    let component = build(&wit, None)?;

    macro_rules! check {
        ($wit_ty:literal, $t:ty, $limit:expr, $a:expr, $b:expr) => {{
            let a = |i: u64| -> $t { $a(i) };
            let b = |i: u64| -> $t { $b(i) };
            let nonzero = |i: u64| -> $t { if b(i) == 0 { 1 } else { b(i) } };
            let expected: [(
                &str,
                fn(u64, &dyn Fn(u64) -> $t, &dyn Fn(u64) -> $t, &dyn Fn(u64) -> $t) -> $t,
            ); 8] = [
                ("add", |i, a, b, _| a(i).wrapping_add(b(i))),
                ("sub", |i, a, b, _| a(i).wrapping_sub(b(i))),
                ("mul", |i, a, b, _| a(i).wrapping_mul(b(i))),
                ("div", |i, a, _, n| a(i).wrapping_div(n(i))),
                ("rem", |i, a, _, n| a(i).wrapping_rem(n(i))),
                ("shl", |i, a, b, _| a(i).wrapping_shl(b(i) as u32)),
                ("shr", |i, a, b, _| a(i).wrapping_shr(b(i) as u32)),
                ("neg", |i, a, _, _| a(i).wrapping_neg()),
            ];
            for (name, f) in expected {
                let export = format!("{name}-{}", $wit_ty);
                let actual = read_streams_until::<$t>(&component, ("", &export), 4096, 1, $limit)?;
                let expected: Vec<$t> =
                    (0..$limit as u64).map(|i| f(i, &a, &b, &nonzero)).collect();
                assert_eq!(actual, vec![expected; 2], "{export}");
            }
        }};
    }
    let mix_a = |i: u64| i.wrapping_mul(0x9E3779B97F4A7C15) ^ (i >> 7);
    let mix_b = |i: u64| i.wrapping_mul(0xD1B54A32D192ED03) ^ (i >> 3);
    check!("u8", u8, 65536, |i: u64| (i >> 8) as u8, |i: u64| i as u8);
    check!(
        "s8",
        i8,
        65536,
        |i: u64| (i >> 8) as u8 as i8,
        |i: u64| i as u8 as i8
    );
    check!("u16", u16, 5000, |i| mix_a(i) as u16, |i| mix_b(i) as u16);
    check!("s16", i16, 5000, |i| mix_a(i) as i16, |i| mix_b(i) as i16);
    check!("u32", u32, 5000, |i| mix_a(i) as u32, |i| mix_b(i) as u32);
    check!("s32", i32, 5000, |i| mix_a(i) as i32, |i| mix_b(i) as i32);
    check!("u64", u64, 5000, mix_a, mix_b);
    check!("s64", i64, 5000, |i| mix_a(i) as i64, |i| mix_b(i) as i64);
    Ok(())
}

#[test]
fn it_wraps_the_minimum_divided_by_minus_one() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @value []
            /// @expression || { let a: i64 = -9223372036854775808; a /% -1 }
            export div: async func() -> stream<s64>;
            /// @value []
            /// @expression || { let a: i64 = -9223372036854775808; a %% -1 }
            export rem: async func() -> stream<s64>;
            /// @value []
            /// @expression || { let a: i32 = -2147483648; a /% -1 }
            export div32: async func() -> stream<s32>;
            /// @value []
            /// @expression || $i as u8 /% 0
            export by-zero: async func() -> stream<u8>;
        }",
        None,
    )?;
    assert_eq!(generated::<i64>(&component, "div", 2)?, vec![i64::MIN; 2]);
    assert_eq!(generated::<i64>(&component, "rem", 2)?, vec![0; 2]);
    assert_eq!(generated::<i32>(&component, "div32", 2)?, vec![i32::MIN; 2]);
    assert_eq!(generated::<u8>(&component, "by-zero", 2)?, Vec::<u8>::new());
    Ok(())
}

#[test]
fn it_rejects_wrapping_operators_on_floats_and_bools() {
    for (ty, expression, expected) in [
        ("f64", "|| $i as f64 +% 1.0", "`+%` isn't supported for f64"),
        ("bool", "|| true *% false", "`*%` isn't supported for bool"),
        ("f32", "|| -%($i as f32)", "`-%` isn't supported for f32"),
        ("f64", "|| $i as f64 <<% 1", "`<<%` isn't supported for f64"),
    ] {
        let wit = format!(
            "package a:b; world w {{ /// @value []\n/// @expression {expression}\nexport f: async func() -> stream<{ty}>; }}"
        );
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }
}

const ARRAYS_WIT: &str = "package a:b;
    world w {
        /// @expression || {
        ///     let z = $init.0 +% ($i / 8 +% 1) *% 0x9E3779B97F4A7C15;
        ///     let z = (z ^ z >> 30) *% 0xBF58476D1CE4E5B9;
        ///     let z = (z ^ z >> 27) *% 0x94D049BB133111EB;
        ///     (z ^ z >> 31).to_le_bytes()
        /// }
        export pseudorandom-bytes: async func(seed: u64) -> stream<u8>;
        /// @value []
        /// @expression || ($i as u32 *% 0x01020304).to_be_bytes()
        export big-endian: async func() -> stream<u8>;
        /// @value []
        /// @expression || ($i as f64 / 8.0).to_le_bytes()
        export float-bytes: async func() -> stream<u8>;
        /// @value [1, 1]
        /// @expression |a, b| [a + b, a + b + b]
        export fibonacci-pairs: async func() -> stream<u32>;
        /// @value []
        /// @expression || [$i, $i + 1, $i + 2]
        export positions: async func() -> stream<u64>;
        /// @value []
        /// @expression || if $i / 2 % 2 == 0 { [1, 2] } else { [3, 4] }
        export alternating: async func() -> stream<u8>;
    }";

#[test]
fn it_generates_several_items_from_an_array() -> Result<()> {
    let component = build(ARRAYS_WIT, None)?;
    for seed in [0u64, 42] {
        let expected: Vec<u8> = (0..1000)
            .flat_map(|n| splitmix(seed, n).to_le_bytes())
            .collect();
        for per_read in [4096, 3] {
            let actual = read_streams_with::<_, u8>(
                &component,
                ("", "pseudorandom-bytes"),
                (seed,),
                per_read,
                2,
                8000,
            )?;
            assert_eq!(
                actual,
                vec![expected.clone(); 4],
                "seed {seed}, {per_read} per read"
            );
        }
    }
    let expected: Vec<u8> = (0..250u64)
        .flat_map(|g| ((g * 4) as u32).wrapping_mul(0x01020304).to_be_bytes())
        .collect();
    assert_eq!(generated::<u8>(&component, "big-endian", 1000)?, expected);
    let expected: Vec<u8> = (0..125u64)
        .flat_map(|g| ((g * 8) as f64 / 8.0).to_le_bytes())
        .collect();
    assert_eq!(generated::<u8>(&component, "float-bytes", 1000)?, expected);
    // a pair is generated whole, or not at all
    let mut expected = vec![1u32, 1];
    loop {
        let (a, b) = (expected[expected.len() - 2], expected[expected.len() - 1]);
        match (
            a.checked_add(b),
            a.checked_add(b).and_then(|c| c.checked_add(b)),
        ) {
            (Some(c), Some(d)) => expected.extend([c, d]),
            _ => break,
        }
    }
    assert_eq!(
        generated::<u32>(&component, "fibonacci-pairs", usize::MAX)?,
        expected
    );
    // `$i` is the position of each group's first item
    assert_eq!(
        generated::<u64>(&component, "positions", 1000)?,
        (0..1000).collect::<Vec<u64>>()
    );
    assert_eq!(
        generated::<u8>(&component, "alternating", 8)?,
        vec![1, 2, 3, 4, 1, 2, 3, 4]
    );
    Ok(())
}

#[test]
fn it_rejects_invalid_arrays() {
    let wit = |ty: &str, expression: &str| {
        format!(
            "package a:b; world w {{ /// @value []\n/// @expression {expression}\nexport f: async func() -> stream<{ty}>; }}"
        )
    };
    for (wit, expected) in [
        (
            wit("u32", "|| $i.to_le_bytes()"),
            "`to_le_bytes` gives bytes, the stream's items must be u8, not u32",
        ),
        (
            wit("u8", "|| 1 + [2]"),
            "an array can only be the closure's result",
        ),
        (
            wit("u8", "|| { let a = [1, 2]; a }"),
            "an array can only be the closure's result",
        ),
        (
            wit("u8", "|| if $i == 0 { [1, 2] } else { [3] }"),
            "the `if` branches give 2 and 1 items",
        ),
        (wit("u8", "|| []"), "an array must have at least one item"),
        (
            wit("u8", "|| $i.to_le_bytes(1)"),
            "`to_le_bytes` takes no arguments",
        ),
        (
            wit("u8", "|| ($i as u8).wrapping_add()"),
            "`wrapping_add` takes an argument",
        ),
        (
            wit("u8", "|| ($i == 0).to_be_bytes()"),
            "`to_be_bytes` isn't supported for bool",
        ),
        (
            wit("u8", "|| [1, 256]"),
            "literal 256 is out of range for u8",
        ),
    ] {
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }
}

const LOOPS_WIT: &str = "package a:b;
    world w {
        /// @value [2]
        /// @expression |p| {
        ///     let mut n = p + 1;
        ///     let mut d: u64 = 2;
        ///     while d * d <= n {
        ///         if n % d == 0 {
        ///             n += 1;
        ///             d = 2;
        ///         } else {
        ///             d += 1;
        ///         }
        ///     }
        ///     n
        /// }
        export primes: async func() -> stream<u64>;
        /// @value []
        /// @expression || {
        ///     let mut n = $i + 1;
        ///     let mut steps: u64 = 0;
        ///     while n != 1 {
        ///         if n % 2 == 0 { n /= 2; } else { n = 3 * n + 1; }
        ///         steps += 1;
        ///     }
        ///     steps
        /// }
        export collatz: async func() -> stream<u64>;
        /// @value []
        /// @expression || {
        ///     let mut f: u64 = 1;
        ///     let mut k: u64 = 1;
        ///     while k <= $i { f *= k; k += 1; }
        ///     f
        /// }
        export factorials: async func() -> stream<u64>;
        /// @value []
        /// @expression || {
        ///     let mut x = $i;
        ///     x *%= 0x9E3779B97F4A7C15;
        ///     x ^= x >> 29;
        ///     x +%= 7;
        ///     x <<%= 67;
        ///     x
        /// }
        export hash: async func() -> stream<u64>;
        /// @value []
        /// @expression || {
        ///     let n = $i + 1;
        ///     let mut sum: u64 = 0;
        ///     let mut d: u64 = 1;
        ///     while d <= n {
        ///         let mut m = n;
        ///         while m >= d { m -= d; }
        ///         if m == 0 { sum += d; }
        ///         d += 1;
        ///     }
        ///     sum
        /// }
        export divisor-sums: async func() -> stream<u64>;
        /// @value []
        /// @expression || {
        ///     let mut sign: i8 = 0;
        ///     let x = $i as i8 - 2;
        ///     if x < 0 { sign = -1; } else if x > 0 { sign = 1; }
        ///     sign
        /// }
        export signs: async func() -> stream<s8>;
    }";

#[test]
fn it_loops() -> Result<()> {
    let component = build(LOOPS_WIT, None)?;
    let is_prime = |n: u64| {
        n >= 2
            && (2..)
                .take_while(|d| d * d <= n)
                .all(|d| !n.is_multiple_of(d))
    };
    let primes: Vec<u64> = (2..).filter(|n| is_prime(*n)).take(2000).collect();
    assert_eq!(generated::<u64>(&component, "primes", 2000)?, primes);
    let collatz = |mut n: u64| {
        let mut steps = 0;
        while n != 1 {
            n = if n.is_multiple_of(2) {
                n / 2
            } else {
                3 * n + 1
            };
            steps += 1;
        }
        steps
    };
    assert_eq!(
        generated::<u64>(&component, "collatz", 500)?,
        (1..=500).map(collatz).collect::<Vec<_>>()
    );
    // overflow inside a loop ends the stream, after 20!
    let factorials: Vec<u64> = (0..=20u64).map(|n| (1..=n).product()).collect();
    assert_eq!(
        generated::<u64>(&component, "factorials", usize::MAX)?,
        factorials
    );
    let hash = |i: u64| {
        let mut x = i.wrapping_mul(0x9E3779B97F4A7C15);
        x ^= x >> 29;
        x.wrapping_add(7).wrapping_shl(67)
    };
    assert_eq!(
        generated::<u64>(&component, "hash", 300)?,
        (0..300).map(hash).collect::<Vec<_>>()
    );
    let divisor_sum = |n: u64| (1..=n).filter(|d| n.is_multiple_of(*d)).sum::<u64>();
    assert_eq!(
        generated::<u64>(&component, "divisor-sums", 60)?,
        (1..=60).map(divisor_sum).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<i8>(&component, "signs", 5)?,
        vec![-1, -1, 0, 1, 1]
    );
    Ok(())
}

#[test]
fn it_rejects_invalid_loops_and_assignments() {
    let wit = |expression: &str| {
        format!(
            "package a:b; world w {{ /// @value [1]\n/// @expression {expression}\nexport f: async func() -> stream<u8>; }}"
        )
    };
    for (expression, expected) in [
        (
            "|n| { let x = n; x = 2; x }",
            "can't assign to `x` at 1:18, declare it with `let mut x`",
        ),
        ("|n| { n = 2; n }", "can't assign to the parameter `n`"),
        (
            "|n| { let mut x = n; x = true; x }",
            "expected u8 for an assignment, found bool",
        ),
        (
            "|n| { let mut x = n; x += 256; x }",
            "literal 256 is out of range for u8",
        ),
        (
            "|n| { while n { } n }",
            "expected bool for a `while` condition, found u8",
        ),
        (
            "|n| { while n > 1 { n } n }",
            "a `while` loop's or an `if` statement's block can't end with a value",
        ),
        (
            "|n| { if n > 1 { n } n }",
            "a `while` loop's or an `if` statement's block can't end with a value",
        ),
        (
            "|n| { let mut x = n; while x > 1 { x -= 1; } }",
            "expected a value at the end of the block",
        ),
        (
            "|n| { if n > 1 { 1 } else { 2 } + 1 }",
            "an `if` with values must be the block's value",
        ),
        ("|n| { y = 1; n }", "unknown name `y`"),
        ("|n| { let mut = n; n }", "expected a name to bind"),
        (
            "|n| { let x = { let mut y = n; y += 1; }; x }",
            "expected a value at the end of the block",
        ),
    ] {
        let wit = wit(expression);
        let err = format!("{:#}", build(&wit, None).expect_err(expression));
        assert!(
            err.contains(expected),
            "{expression}: expected {expected:?} in {err:?}"
        );
    }
}

const POW_WIT: &str = "package a:b;
    world w {
        /// @value []
        /// @expression || (3 as u64) ** ($i as u32)
        export powers-u64: async func() -> stream<u64>;
        /// @value []
        /// @expression || (-3 as i64) ** ($i as u32)
        export powers-s64: async func() -> stream<s64>;
        /// @value []
        /// @expression || (2 as u8) ** ($i as u32)
        export powers-u8: async func() -> stream<u8>;
        /// @value []
        /// @expression || (-2 as i8) ** ($i as u32)
        export powers-s8: async func() -> stream<s8>;
        /// @value []
        /// @expression || (7 as u32) ** ($i as u32)
        export powers-u32: async func() -> stream<u32>;
        /// @value []
        /// @expression || (-5 as i16) ** ($i as u32)
        export powers-s16: async func() -> stream<s16>;
        /// @value []
        /// @expression || (1.5 as f64) ** ($i as i32 - 40)
        export powi-f64: async func() -> stream<f64>;
        /// @value []
        /// @expression || (10 as f32) ** ($i as i32)
        export powi-f32: async func() -> stream<f32>;
        /// @value []
        /// @expression || { let x = $i as i32; -x ** 2 }
        export negated: async func() -> stream<s32>;
        /// @value []
        /// @expression || (2 as u32) ** 3 ** ($i as u32)
        export right-associative: async func() -> stream<u32>;
        /// @value []
        /// @expression || { let mut x = ($i + 2) as u32; x **= 2; x **%= 3; x }
        export compound: async func() -> stream<u32>;
        /// @expression || $init.0 * ($init.1 ** $i)
        export geometric: async func(n: u64, r: u64) -> stream<u64>;
        /// @value []
        /// @expression || (2 as i64) ** ($i as i64 - 1)
        export negative-exponent: async func() -> stream<s64>;
        /// @value []
        /// @expression || (2 as f64) ** ($i as i64 - 3)
        export float-wide-exponent: async func() -> stream<f64>;
        /// @value []
        /// @expression || (1 as u8) ** ($i * 0x1_0000_0001)
        export huge-exponent: async func() -> stream<u8>;
    }";

#[test]
fn it_raises_to_powers_as_rust() -> Result<()> {
    let c = build(POW_WIT, None)?;
    // checked powers end the stream at the first that overflows
    fn powers<T: Copy>(base: T, pow: impl Fn(T, u32) -> Option<T>) -> Vec<T> {
        (0..).map_while(|e| pow(base, e)).collect()
    }
    let all = usize::MAX;
    assert_eq!(
        generated::<u64>(&c, "powers-u64", all)?,
        powers(3u64, u64::checked_pow)
    );
    assert_eq!(
        generated::<i64>(&c, "powers-s64", all)?,
        powers(-3i64, i64::checked_pow)
    );
    assert_eq!(
        generated::<u8>(&c, "powers-u8", all)?,
        powers(2u8, u8::checked_pow)
    );
    assert_eq!(
        generated::<i8>(&c, "powers-s8", all)?,
        powers(-2i8, i8::checked_pow)
    );
    assert_eq!(
        generated::<u32>(&c, "powers-u32", all)?,
        powers(7u32, u32::checked_pow)
    );
    assert_eq!(
        generated::<i16>(&c, "powers-s16", all)?,
        powers(-5i16, i16::checked_pow)
    );

    // floats, as `powi`, within rounding, the stream ends when a power isn't finite
    let close = |actual: &[f64], expected: &[f64]| {
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() <= e.abs() * 1e-12, "{a} != {e}");
        }
    };
    let actual = generated::<f64>(&c, "powi-f64", 100)?;
    let expected: Vec<f64> = (0..100).map(|i| 1.5f64.powi(i - 40)).collect();
    close(&actual, &expected);
    let actual: Vec<f64> = generated::<f32>(&c, "powi-f32", all)?
        .into_iter()
        .map(f64::from)
        .collect();
    let expected: Vec<f64> = (0..)
        .map(|i| 10f32.powi(i))
        .take_while(|x| x.is_finite())
        .map(f64::from)
        .collect();
    close(&actual, &expected);

    // `**` binds tighter than a unary operator on its left, and associates right
    assert_eq!(
        generated::<i32>(&c, "negated", 5)?,
        (0..5).map(|x: i32| -(x * x)).collect::<Vec<_>>()
    );
    assert_eq!(
        generated::<u32>(&c, "right-associative", 3)?,
        vec![2, 8, 512]
    );
    // the exponent may be any integer
    let geometric =
        read_streams_with::<_, u64>(&c, ("", "geometric"), (5u64, 3u64), 64, 1, usize::MAX)?;
    let expected: Vec<u64> = (0..)
        .map_while(|e| 3u64.checked_pow(e).and_then(|p| 5u64.checked_mul(p)))
        .collect();
    assert_eq!(geometric, vec![expected; 2]);
    // an integer's negative power ends the stream
    assert_eq!(
        generated::<i64>(&c, "negative-exponent", 3)?,
        Vec::<i64>::new()
    );
    assert_eq!(
        generated::<f64>(&c, "float-wide-exponent", 6)?,
        vec![0.125, 0.25, 0.5, 1.0, 2.0, 4.0]
    );
    // exponents past u32
    assert_eq!(generated::<u8>(&c, "huge-exponent", 3)?, vec![1, 1, 1]);
    assert_eq!(
        generated::<u32>(&c, "compound", 50)?,
        (0..50u32)
            .map(|i| ((i + 2) * (i + 2)).wrapping_pow(3))
            .collect::<Vec<_>>()
    );
    Ok(())
}

/// `**%` on every integer type, against Rust's `wrapping_pow`: every base and
/// exponent below 256 for 8-bit types, pseudorandom pairs for wider types.
#[test]
fn it_wraps_powers_as_rust() -> Result<()> {
    let types = [
        ("u8", "u8", true),
        ("s8", "i8", true),
        ("u16", "u16", false),
        ("s32", "i32", false),
        ("u64", "u64", false),
        ("s64", "i64", false),
    ];
    let mut wit = String::from("package a:b; world w {\n");
    for (wit_ty, rust_ty, exhaustive) in types {
        let base = match exhaustive {
            true => format!("($i >> 8) as u8 as {rust_ty}"),
            false => format!("($i *% 0x9E3779B97F4A7C15 ^ $i >> 7) as {rust_ty}"),
        };
        wit.push_str(&format!(
            "/// @value []\n/// @expression || ({base}) **% ($i as u8 as u32)\n\
             export pow-{wit_ty}: async func() -> stream<{wit_ty}>;\n"
        ));
    }
    wit.push('}');
    let component = build(&wit, None)?;
    let mix = |i: u64| i.wrapping_mul(0x9E3779B97F4A7C15) ^ (i >> 7);
    macro_rules! check {
        ($wit_ty:literal, $t:ty, $limit:expr, $base:expr) => {{
            let export = format!("pow-{}", $wit_ty);
            let actual = read_streams_until::<$t>(&component, ("", &export), 4096, 1, $limit)?;
            let expected: Vec<$t> = (0..$limit as u64)
                .map(|i| ($base(i) as $t).wrapping_pow(i as u8 as u32))
                .collect();
            assert_eq!(actual, vec![expected; 2], "{export}");
        }};
    }
    check!("u8", u8, 65536, |i: u64| (i >> 8) as u8);
    check!("s8", i8, 65536, |i: u64| (i >> 8) as u8 as i8);
    check!("u16", u16, 5000, mix);
    check!("s32", i32, 5000, mix);
    check!("u64", u64, 5000, mix);
    check!("s64", i64, 5000, mix);
    Ok(())
}

#[test]
fn it_rejects_invalid_powers() {
    let wit = |ty: &str, expression: &str| {
        format!(
            "package a:b; world w {{ /// @value []\n/// @expression {expression}\nexport f: async func() -> stream<{ty}>; }}"
        )
    };
    for (wit, expected) in [
        (
            wit("u64", "|| $i ** ($i as f64)"),
            "the exponent of `**` must be an integer, found f64",
        ),
        (
            wit("f64", "|| ($i as f64) ** ($i == 0)"),
            "the exponent of `**` must be an integer, found bool",
        ),
        (wit("u64", "|| $i ** 1.5"), "expected u32, found float 1.5"),
        (
            wit("f64", "|| ($i as f64) **% 2"),
            "`**%` isn't supported for f64",
        ),
        (wit("bool", "|| true ** 2"), "`**` expects numeric operands"),
        (wit("u64", "|| $i as u64 ** 2"), "unexpected `**`"),
        (wit("u64", "|| $i **"), "expected an expression at the end"),
    ] {
        let err = format!("{:#}", build(&wit, None).expect_err(&wit));
        assert!(err.contains(expected), "expected {expected:?} in {err:?}");
    }
}

#[test]
fn it_reads_seed_fields_by_name() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @expression || $init.n * ($init.r ** $i)
            export geometric: async func(n: u64, r: u64) -> stream<u64>;
            /// @expression || $init.start_value + $init.1 * $i
            export arithmetic: async func(start-value: u64, step: u64) -> stream<u64>;
            /// @expression || {
            ///     let mut total = $init.base;
            ///     let mut k: u64 = 0;
            ///     while k < $i { total += $init.step; k += 1; }
            ///     total
            /// }
            export looped: async func(base: u64, step: u64) -> stream<u64>;
        }",
        None,
    )?;
    let geometric =
        read_streams_with::<_, u64>(&component, ("", "geometric"), (5u64, 3u64), 64, 1, 4)?;
    assert_eq!(geometric, vec![vec![5, 15, 45, 135]; 2]);
    let arithmetic =
        read_streams_with::<_, u64>(&component, ("", "arithmetic"), (10u64, 4u64), 64, 1, 4)?;
    assert_eq!(arithmetic, vec![vec![10, 14, 18, 22]; 2]);
    let looped = read_streams_with::<_, u64>(&component, ("", "looped"), (1u64, 2u64), 64, 1, 4)?;
    assert_eq!(looped, vec![vec![1, 3, 5, 7]; 2]);

    let err = format!(
        "{:#}",
        build(
            "package a:b; world w {
                /// @expression || $init.m * $i
                export f: async func(n: u64, start-value: u64) -> stream<u64>;
            }",
            None,
        )
        .expect_err("unknown field")
    );
    assert!(
        err.contains(
            "`$init` has no field `m` at 1:10, the function's parameters are `n`, `start_value`"
        ),
        "{err}"
    );
    Ok(())
}

#[test]
fn it_reads_arguments_by_name() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @expression || n * (r ** $i)
            export geometric: async func(n: u64, r: u64) -> stream<u64>;
            /// @expression || { let n = n + 1; n * $i }
            export shadowed: async func(n: u64) -> stream<u64>;
            /// @expression || start_value + $i
            export kebab: async func(start-value: u64) -> stream<u64>;
            /// @value [0]
            /// @expression |v| v + step
            export stepping: async func(step: u64) -> stream<u64>;
            /// @expression |n| n + 1
            export items: async func(n: u64) -> stream<u64>;
        }",
        None,
    )?;
    let read = |name: &str, args: (u64,)| {
        read_streams_with::<_, u64>(&component, ("", name), args, 64, 1, 4)
    };
    let geometric =
        read_streams_with::<_, u64>(&component, ("", "geometric"), (5u64, 3u64), 64, 1, 4)?;
    assert_eq!(geometric, vec![vec![5, 15, 45, 135]; 2]);
    // a `let` shadows an argument, which is never changed
    assert_eq!(read("shadowed", (2,))?, vec![vec![0, 3, 6, 9]; 2]);
    assert_eq!(read("kebab", (10,))?, vec![vec![10, 11, 12, 13]; 2]);
    // read by name, the arguments aren't items, the stream starts from `@value`
    assert_eq!(read("stepping", (5,))?, vec![vec![0, 5, 10, 15]; 2]);
    // a closure parameter shadows an argument, the arguments are the first items
    assert_eq!(read("items", (7,))?, vec![vec![7, 8, 9, 10]; 2]);

    let err = format!(
        "{:#}",
        build(
            "package a:b; world w {
                /// @expression || { n = 1; n }
                export f: async func(n: u64) -> stream<u64>;
            }",
            None,
        )
        .expect_err("assigned argument")
    );
    assert!(
        err.contains("can't assign to the argument `n` at 1:6, arguments never change"),
        "{err}"
    );
    Ok(())
}

#[test]
fn it_rejects_shadowing_or_assigning_builtin_variables() {
    for (expression, expected) in [
        ("|n| { let $i = 1; n }", "`$i` can't be bound at 1:11"),
        (
            "|n| { let mut $init = 1; n }",
            "`$init` can't be bound at 1:15",
        ),
        ("|$i| $i", "`$i` can't be bound at 1:2"),
        ("|n| { $i = 1; n }", "`$i` can't be assigned at 1:7"),
        ("|n| { $i += 1; n }", "`$i` can't be assigned at 1:7"),
        ("|n| { $init.0 = 1; n }", "`$init` can't be assigned at 1:7"),
        (
            "|n| { $init.x *%= 2; n }",
            "`$init` can't be assigned at 1:7",
        ),
    ] {
        let wit = format!(
            "package a:b; world w {{ /// @value [1]\n/// @expression {expression}\nexport f: async func(x: u64) -> stream<u64>; }}"
        );
        let err = format!("{:#}", build(&wit, None).expect_err(expression));
        assert!(
            err.contains(expected),
            "{expression}: expected {expected:?} in {err:?}"
        );
    }
}

#[test]
fn it_never_changes_seed_or_position() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @expression || {
            ///     let mut copy = $i;
            ///     copy += 100;
            ///     let mut seed = $init.0;
            ///     seed *= 3;
            ///     let mut n = n;
            ///     n += 1;
            ///     [$i, $init.0, $init.n, n, copy + seed]
            /// }
            export copies: async func(n: u64) -> stream<u64>;
        }",
        None,
    )?;
    // each evaluation sees the arguments unchanged, and its own position, however
    // its copies changed, here and in earlier evaluations
    let items = read_streams_with::<_, u64>(&component, ("", "copies"), (7u64,), 64, 1, 15)?;
    let expected: Vec<u64> = (0..3u64)
        .flat_map(|g| {
            let i = g * 5;
            [i, 7, 7, 8, i + 100 + 21]
        })
        .collect();
    assert_eq!(items, vec![expected; 2]);
    Ok(())
}

#[test]
fn it_reads_the_closures_parameters_from_call() -> Result<()> {
    let component = build(
        "package a:b; world w {
            /// @value [1, 1]
            /// @expression |a, b| { let a: u64 = 100; $call.a + b + a - 100 }
            export shadowed: async func() -> stream<u64>;
            /// @value [1, 1]
            /// @expression |_, b| $call.0 + b
            export unnamed: async func() -> stream<u64>;
            /// @value [1, 1]
            /// @expression |a, b| a + $call.b
            export named: async func() -> stream<u64>;
        }",
        None,
    )?;
    let fibonacci = vec![1u64, 1, 2, 3, 5, 8, 13, 21];
    for name in ["shadowed", "unnamed", "named"] {
        assert_eq!(generated::<u64>(&component, name, 8)?, fibonacci, "{name}");
    }

    for (expression, expected) in [
        (
            "|a| $call.1",
            "`$call.1` doesn't exist at 1:11, `$call` has 1 fields",
        ),
        (
            "|a, _| $call.b",
            "`$call` has no field `b` at 1:14, the closure's parameters are `a`",
        ),
        (
            "|| $call.0",
            "`$call.0` doesn't exist at 1:10, `$call` has 0 fields",
        ),
        (
            "|a| $call",
            "expected `.` and a field, `$call` is the closure's parameters",
        ),
        ("|a| { let $call = 1; a }", "`$call` can't be bound at 1:11"),
        ("|a| { $call.a = 1; a }", "`$call` can't be assigned at 1:7"),
        (
            "|a| $args.0",
            "unknown variable `$args` at 1:5, the variables are `$init`, `$call` and `$i`",
        ),
    ] {
        let wit = format!(
            "package a:b; world w {{ /// @value [1]\n/// @expression {expression}\nexport f: async func() -> stream<u64>; }}"
        );
        let err = format!("{:#}", build(&wit, None).expect_err(expression));
        assert!(
            err.contains(expected),
            "{expression}: expected {expected:?} in {err:?}"
        );
    }
    Ok(())
}
