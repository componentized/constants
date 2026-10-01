//! Helpers for tests that run components, the CLI's generated components and those built into
//! `target/components/` by make.

use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, Val},
};

/// Calls each named export (`interface` may be empty for world-level
/// functions) and returns the results. Type-only `imports` are satisfied with
/// empty instances.
pub fn call(bytes: &[u8], imports: &[&str], calls: &[(&str, &str)]) -> Result<Vec<Val>> {
    call_with(
        bytes,
        |linker| {
            for import in imports {
                linker.instance(import)?;
            }
            Ok(())
        },
        calls,
    )
}

/// Like [`call`], with imports defined by `link`.
pub fn call_with(
    bytes: &[u8],
    link: impl FnOnce(&mut Linker<()>) -> Result<()>,
    calls: &[(&str, &str)],
) -> Result<Vec<Val>> {
    let mut config = Config::new();
    config.wasm_component_model_fixed_length_lists(true);
    // types that values can't reach may still appear in result types
    config.wasm_component_model_async(true);
    config.wasm_component_model_error_context(true);
    config.wasm_component_model_map(true);
    let engine = Engine::new(&config)?;
    let mut linker = Linker::<()>::new(&engine);
    link(&mut linker)?;
    let mut store = Store::new(&engine, ());
    let component = Component::from_binary(&engine, bytes)?;
    let instance = linker.instantiate(&mut store, &component)?;

    let mut results = vec![];
    for (interface, name) in calls {
        let parent = match *interface {
            "" => None,
            interface => Some(
                instance
                    .get_export_index(&mut store, None, interface)
                    .expect("interface export"),
            ),
        };
        let index = instance
            .get_export_index(&mut store, parent.as_ref(), name)
            .unwrap_or_else(|| panic!("missing export {name}"));
        let func = instance.get_func(&mut store, index).expect("func export");
        let mut result = [Val::Bool(false)];
        func.call(&mut store, &[], &mut result)?;
        results.push(result[0].clone());
    }
    Ok(results)
}

/// Root directory of the workspace.
pub fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace dir")
}

/// Directory containing the built components.
pub fn components_dir() -> PathBuf {
    workspace_dir().join("target/components")
}

/// Path to the named component in `target/components/`, e.g. `factory`, rebuilt with make unless
/// it was already built by this process.
pub fn built_component(name: &str) -> Result<PathBuf> {
    static BUILT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    // hold the lock while building so concurrent tests don't run make over each other
    let mut built = BUILT.lock().unwrap_or_else(|err| err.into_inner());
    let built = built.get_or_insert_with(HashSet::new);
    // the make target for a component, its files are named by absolute path
    let target = format!("components/{name}");
    if !built.contains(name) {
        let output = Command::new("make")
            .arg("-C")
            .arg(workspace_dir())
            .arg(&target)
            .output()
            .context("failed to run make")?;
        if !output.status.success() {
            bail!(
                "failed to build {target}:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        built.insert(name.to_string());
    }
    Ok(components_dir().join(name).join(format!("{name}.wasm")))
}
