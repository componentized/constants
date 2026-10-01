//! Exercises the config component built by make into `target/components/config/config.wasm`,
//! with the `config-values` it imports provided by the host.

use anyhow::{Context, Result, bail};
use test_harness::built_component;
use wasmtime::{
    Engine, ResourceLimiter, Store,
    component::{Component, Instance, Linker, Val},
};

const VALUES_INTERFACE: &str = "componentized:constants/config-values@0.1.0-dev";
const STORE_INTERFACE: &str = "wasi:config/store@0.2.0-rc.1";

/// An instance of the config component, importing `values` as the config values.
struct Config {
    store: Store<MemoryLimiter>,
    instance: Instance,
}

/// Records the size the component's memory has grown to.
#[derive(Default)]
struct MemoryLimiter {
    memory_size: usize,
}

impl ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.memory_size = self.memory_size.max(desired);
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

impl Config {
    fn new(values: Vec<(String, String)>) -> Result<Self> {
        let engine = Engine::default();
        let path = built_component("config")?;
        let bytes =
            std::fs::read(&path).with_context(|| format!("unable to read {}", path.display()))?;
        let component = Component::from_binary(&engine, &bytes)?;

        let values = Val::List(values.iter().map(|(k, v)| pair(k, v)).collect());
        let mut linker = Linker::<MemoryLimiter>::new(&engine);
        linker
            .instance(VALUES_INTERFACE)?
            .func_new("values", move |_, _, _, results| {
                results[0] = values.clone();
                Ok(())
            })?;

        let mut store = Store::new(&engine, MemoryLimiter::default());
        store.limiter(|limiter| limiter);
        let instance = linker.instantiate(&mut store, &component)?;
        Ok(Self { store, instance })
    }

    /// Calls a `wasi:config/store` function.
    fn call(&mut self, name: &str, params: Vec<Val>) -> Result<Val> {
        let parent = self
            .instance
            .get_export_index(&mut self.store, None, STORE_INTERFACE)
            .with_context(|| format!("missing export {STORE_INTERFACE}"))?;
        let index = self
            .instance
            .get_export_index(&mut self.store, Some(&parent), name)
            .with_context(|| format!("missing export {name}"))?;
        let func = self
            .instance
            .get_func(&mut self.store, index)
            .with_context(|| format!("{name} is not a function"))?;
        let mut results = [Val::Bool(false)];
        func.call(&mut self.store, &params, &mut results)?;
        Ok(results[0].clone())
    }

    /// The size the component's memory has grown to, in bytes.
    fn memory_size(&self) -> usize {
        self.store.data().memory_size
    }
}

/// Calls a `wasi:config/store` function on a new instance of the config component.
fn call(values: &[(&str, &str)], name: &str, params: Vec<Val>) -> Result<Val> {
    let values = values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Config::new(values)?.call(name, params)
}

fn get(values: &[(&str, &str)], key: &str) -> Result<Option<String>> {
    match ok(call(values, "get", vec![Val::String(key.into())])?)? {
        Val::Option(None) => Ok(None),
        Val::Option(Some(value)) => match *value {
            Val::String(value) => Ok(Some(value)),
            other => bail!("unexpected value {other:?}"),
        },
        other => bail!("unexpected option {other:?}"),
    }
}

fn get_all(values: &[(&str, &str)]) -> Result<Val> {
    ok(call(values, "get-all", vec![])?)
}

/// Unwraps `result<T, error>`, failing on an error.
fn ok(result: Val) -> Result<Val> {
    match result {
        Val::Result(Ok(Some(value))) => Ok(*value),
        other => bail!("expected ok, got {other:?}"),
    }
}

fn pair(key: &str, value: &str) -> Val {
    Val::Tuple(vec![Val::String(key.into()), Val::String(value.into())])
}

const VALUES: &[(&str, &str)] = &[("host", "localhost"), ("port", "8080")];

#[test]
fn it_gets_a_value() -> Result<()> {
    assert_eq!(get(VALUES, "port")?, Some("8080".into()));
    Ok(())
}

#[test]
fn it_gets_none_for_a_missing_key() -> Result<()> {
    assert_eq!(get(VALUES, "missing")?, None);
    Ok(())
}

#[test]
fn it_gets_the_last_value_for_a_duplicate_key() -> Result<()> {
    let values = &[("mode", "first"), ("mode", "second"), ("other", "value")];
    assert_eq!(get(values, "mode")?, Some("second".into()));
    Ok(())
}

#[test]
fn it_gets_all_values_in_order() -> Result<()> {
    assert_eq!(
        get_all(VALUES)?,
        Val::List(vec![pair("host", "localhost"), pair("port", "8080")])
    );
    Ok(())
}

#[test]
fn it_serves_empty_values() -> Result<()> {
    assert_eq!(get(&[], "host")?, None);
    assert_eq!(get_all(&[])?, Val::List(vec![]));
    Ok(())
}

#[test]
fn it_reuses_memory_across_calls() -> Result<()> {
    // about 100KiB of values per call, more than the component's initial memory
    let values: Vec<(String, String)> = (0..100)
        .map(|i| (format!("key-{i}"), format!("{i}-").repeat(250)))
        .collect();
    let expected = Val::List(values.iter().map(|(k, v)| pair(k, v)).collect());
    let mut config = Config::new(values)?;

    assert_eq!(ok(config.call("get-all", vec![])?)?, expected);
    let memory_size = config.memory_size();
    assert!(memory_size > 0, "values should grow memory");

    // memory freed by one call is reused by the next, without corrupting the values returned
    for i in 0..200 {
        assert_eq!(ok(config.call("get-all", vec![])?)?, expected, "call {i}");
        let key = format!("key-{}", i % 100);
        assert_eq!(
            ok(config.call("get", vec![Val::String(key)])?)?,
            Val::Option(Some(Box::new(Val::String(
                format!("{}-", i % 100).repeat(250)
            )))),
            "call {i}"
        );
    }
    assert_eq!(
        config.memory_size(),
        memory_size,
        "memory grew across calls"
    );
    Ok(())
}
