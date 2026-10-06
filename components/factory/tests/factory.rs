//! Exercises the factory component built by make into `target/components/factory/factory.wasm`,
//! along with `target/components/dep-extract-wit/dep-extract-wit.wasm` from
//! `componentized:component`, which make also fetches.

use anyhow::{Context, Result, anyhow, bail};
use std::sync::OnceLock;
use test_harness::{built_component, call, workspace_dir};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, Val},
};

const TYPES: &str = "componentized:component/types@0.0.0-dev";
const WIT_INTERFACE: &str = "componentized:component/wit@0.0.0-dev";
const FACTORY_INTERFACE: &str = "componentized:constants/factory@0.1.1-dev";

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.wasm_component_model_async(true);
        config.wasm_component_model_map(true);
        Engine::new(&config).expect("engine")
    })
}

/// Compiles each component once, shared by every test.
fn load(
    name: &'static str,
    cache: &'static OnceLock<Result<Component, String>>,
) -> Result<&'static Component> {
    cache
        .get_or_init(|| {
            let compile = || -> Result<_> {
                let path = built_component(name)?;
                let bytes = std::fs::read(&path)
                    .with_context(|| format!("unable to read {}", path.display()))?;
                Ok(Component::from_binary(engine(), &bytes)?)
            };
            compile().map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

fn factory() -> Result<&'static Component> {
    static CACHE: OnceLock<Result<Component, String>> = OnceLock::new();
    load("factory", &CACHE)
}

fn extract_wit() -> Result<&'static Component> {
    static CACHE: OnceLock<Result<Component, String>> = OnceLock::new();
    load("dep-extract-wit", &CACHE)
}

/// Calls an async exported function, satisfying type-only imports with empty
/// instances.
fn call_async(
    component: &Component,
    imports: &[&str],
    interface: &str,
    name: &str,
    params: Vec<Val>,
) -> Result<Val> {
    let mut linker = Linker::<()>::new(engine());
    for import in imports {
        linker.instance(import)?;
    }
    let mut store = Store::new(engine(), ());
    futures::executor::block_on(async {
        let instance = linker.instantiate_async(&mut store, component).await?;
        let parent = instance
            .get_export_index(&mut store, None, interface)
            .with_context(|| format!("missing export {interface}"))?;
        let index = instance
            .get_export_index(&mut store, Some(&parent), name)
            .with_context(|| format!("missing export {name}"))?;
        let func = instance
            .get_func(&mut store, index)
            .with_context(|| format!("{name} is not a function"))?;
        let result = store
            .run_concurrent(async |accessor| {
                let mut results = [Val::Bool(false)];
                func.call_concurrent(accessor, &params, &mut results)
                    .await?;
                wasmtime::Result::<_>::Ok(results[0].clone())
            })
            .await??;
        Ok(result)
    })
}

/// Unwraps `result<T, error>`, returning the error's message.
fn unwrap_result(result: Val) -> Result<Result<Val, String>> {
    match result {
        Val::Result(Ok(Some(value))) => Ok(Ok(*value)),
        Val::Result(Err(Some(err))) => match *err {
            Val::Variant(case, Some(message)) if case == "other" => match *message {
                Val::Option(Some(message)) => match *message {
                    Val::String(message) => Ok(Err(message)),
                    other => bail!("unexpected message {other:?}"),
                },
                other => bail!("unexpected message {other:?}"),
            },
            other => bail!("unexpected error {other:?}"),
        },
        other => bail!("unexpected result {other:?}"),
    }
}

fn bytes_val(bytes: Vec<u8>) -> Val {
    Val::List(bytes.into_iter().map(Val::U8).collect())
}

fn val_bytes(val: Val) -> Result<Vec<u8>> {
    match val {
        Val::List(bytes) => bytes
            .into_iter()
            .map(|b| match b {
                Val::U8(b) => Ok(b),
                other => bail!("unexpected byte {other:?}"),
            })
            .collect(),
        other => bail!("unexpected component {other:?}"),
    }
}

enum Wit<'a> {
    Text(&'a str),
    Encoded(Vec<u8>),
    Parsed(Val),
}

/// Calls the factory's `create` function, returning the created component or
/// the error message.
fn create(
    wit: Wit,
    world: Option<&str>,
    overrides: Option<&str>,
) -> Result<Result<Vec<u8>, String>> {
    let string = |s: Option<&str>| Val::Option(s.map(|s| Box::new(Val::String(s.into()))));
    let wave = |s: Option<&str>| {
        Val::Option(s.map(|s| {
            Box::new(Val::Variant(
                "wave".into(),
                Some(Box::new(Val::String(s.into()))),
            ))
        }))
    };
    let wit = match wit {
        Wit::Text(text) => Val::Variant("text".into(), Some(Box::new(Val::String(text.into())))),
        Wit::Encoded(bytes) => Val::Variant("encoded".into(), Some(Box::new(bytes_val(bytes)))),
        Wit::Parsed(wit) => Val::Variant("parsed".into(), Some(Box::new(wit))),
    };
    let result = call_async(
        factory()?,
        &[TYPES, WIT_INTERFACE],
        FACTORY_INTERFACE,
        "create",
        vec![wit, string(world), wave(overrides)],
    )?;
    match unwrap_result(result)? {
        Ok(component) => Ok(Ok(val_bytes(component)?)),
        Err(message) => Ok(Err(message)),
    }
}

/// Extracts the WIT from a component or encoded WIT package with
/// `componentized:component/wit#extract`.
fn extract(bytes: Vec<u8>) -> Result<Val> {
    let result = call_async(
        extract_wit()?,
        &[TYPES],
        WIT_INTERFACE,
        "extract",
        vec![bytes_val(bytes)],
    )?;
    unwrap_result(result)?.map_err(|e| anyhow!(e))
}

fn encode(wit: &str) -> Result<Vec<u8>> {
    let mut resolve = wit_parser::Resolve::default();
    let pkg = match wit.starts_with("package ") || wit.starts_with('\n') {
        true => resolve.push_str("input.wit", wit)?,
        false => resolve.push_path(workspace_dir().join(wit))?.0,
    };
    wit_component::encode(&resolve, pkg, true)
}

const WIT: &str = r#"
package example:factory;

interface types {
    record point { x: s32, y: s32 }
}

interface constants {
    use types.{point};

    /// @value "hello"
    greeting: func() -> string;

    /// @value {x: 1, y: 2}
    origin: func() -> point;
}

world config {
    export constants;
}

world other {
    /// @value 42
    export answer: func() -> u32;
}
"#;

fn string(s: &str) -> Val {
    Val::String(s.into())
}

fn point(x: i32, y: i32) -> Val {
    Val::Record(vec![("x".into(), Val::S32(x)), ("y".into(), Val::S32(y))])
}

fn call_config(component: &[u8]) -> Result<Vec<Val>> {
    call(
        component,
        &["example:factory/types"],
        &[
            ("example:factory/constants", "greeting"),
            ("example:factory/constants", "origin"),
        ],
    )
}

#[test]
fn it_creates_components_from_wit_text() -> Result<()> {
    let component = create(Wit::Text(WIT), Some("config"), None)?.map_err(|e| anyhow!(e))?;
    assert_eq!(call_config(&component)?, vec![string("hello"), point(1, 2)]);
    Ok(())
}

#[test]
fn it_creates_components_from_encoded_wit() -> Result<()> {
    let component = create(
        Wit::Encoded(encode(WIT)?),
        Some("other"),
        Some("{answer: 7}"),
    )?
    .map_err(|e| anyhow!(e))?;
    let actual = call(&component, &[], &[("", "answer")])?;
    assert_eq!(actual, vec![Val::U32(7)]);
    Ok(())
}

#[test]
fn it_creates_components_from_parsed_wit() -> Result<()> {
    let wit = extract(encode(WIT)?)?;
    let component = create(Wit::Parsed(wit), Some("config"), None)?.map_err(|e| anyhow!(e))?;
    assert_eq!(call_config(&component)?, vec![string("hello"), point(1, 2)]);
    Ok(())
}

#[test]
fn it_creates_identical_components_from_encoded_and_parsed_wit() -> Result<()> {
    // exercises every supported type, including aliases and imported types
    let encoded = encode("crates/componentized-constants/tests/fixtures/all")?;
    let from_encoded =
        create(Wit::Encoded(encoded.clone()), None, None)?.map_err(|e| anyhow!(e))?;
    let from_parsed =
        create(Wit::Parsed(extract(encoded)?), None, None)?.map_err(|e| anyhow!(e))?;
    assert!(from_encoded == from_parsed, "components differ");
    Ok(())
}

#[test]
fn it_reimplements_the_world_of_a_parsed_component() -> Result<()> {
    // components don't carry docs, so every value comes from the overrides
    let original = create(Wit::Text(WIT), Some("config"), None)?.map_err(|e| anyhow!(e))?;
    let wit = extract(original)?;
    let component = create(
        Wit::Parsed(wit),
        None,
        Some(r#"{constants: {greeting: "hi", origin: {x: 3, y: 4}}}"#),
    )?
    .map_err(|e| anyhow!(e))?;
    assert_eq!(call_config(&component)?, vec![string("hi"), point(3, 4)]);
    Ok(())
}

#[test]
fn it_returns_errors() -> Result<()> {
    let err = create(Wit::Text(WIT), None, None)?.expect_err("multiple worlds");
    assert!(err.contains("multiple worlds"), "{err}");

    let err =
        create(Wit::Text(WIT), Some("other"), Some("{answer: -1}"))?.expect_err("invalid override");
    assert!(err.contains("invalid override for `answer`"), "{err}");

    let err = create(Wit::Encoded(vec![0, 1, 2]), None, None)?.expect_err("invalid wasm");
    assert!(!err.is_empty());

    let original = create(Wit::Text(WIT), Some("config"), None)?.map_err(|e| anyhow!(e))?;
    let err = create(Wit::Parsed(extract(original)?), None, None)?.expect_err("missing values");
    assert!(
        err.contains("missing value for `example:factory/constants#greeting`"),
        "{err}"
    );
    Ok(())
}
