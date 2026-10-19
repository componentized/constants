//! Helpers for tests that run components, the CLI's generated components and those built into
//! `target/components/` by make.

use anyhow::{Context, Result, bail};
use futures::channel::oneshot;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
};
use wasmtime::{
    Config, Engine, Store, StoreContextMut,
    component::{
        Component, ComponentNamedList, Func, Instance, Lift, Linker, Lower, Source, StreamConsumer,
        StreamReader, StreamResult, Val,
    },
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
    let (mut store, instance) = instantiate(bytes, link)?;
    let mut results = vec![];
    for (interface, name) in calls {
        let func = export(&mut store, &instance, interface, name);
        let mut result = [Val::Bool(false)];
        func.call(&mut store, &[], &mut result)?;
        results.push(result[0].clone());
    }
    Ok(results)
}

/// Calls the named export, which returns a `stream<T>`, and reads the stream
/// until the writer closes it. Each read takes at most `per_read` items, so a
/// writer sees reads that take only some of the items it offers.
///
/// The export is called `calls` times at once, then as many times again once
/// those streams are closed, returning the items of each stream in call order.
pub fn read_streams<T: Lift + Send + Sync + 'static>(
    bytes: &[u8],
    export: (&str, &str),
    per_read: usize,
    calls: usize,
) -> Result<Vec<Vec<T>>> {
    read_streams_until(bytes, export, per_read, calls, usize::MAX)
}

/// Like [`read_streams`], closing each stream once `limit` items are read.
pub fn read_streams_until<T: Lift + Send + Sync + 'static>(
    bytes: &[u8],
    export: (&str, &str),
    per_read: usize,
    calls: usize,
    limit: usize,
) -> Result<Vec<Vec<T>>> {
    read_streams_with(bytes, export, (), per_read, calls, limit)
}

/// Like [`read_streams_until`], calling the export with `params`.
pub fn read_streams_with<P, T>(
    bytes: &[u8],
    (interface, name): (&str, &str),
    params: P,
    per_read: usize,
    calls: usize,
    limit: usize,
) -> Result<Vec<Vec<T>>>
where
    P: ComponentNamedList + Lower + Clone + Send + Sync + 'static,
    T: Lift + Send + Sync + 'static,
{
    let (mut store, instance) = instantiate(bytes, |_| Ok(()))?;
    let func = export(&mut store, &instance, interface, name);
    let func = func.typed::<P, (StreamReader<T>,)>(&store)?;
    let streams =
        futures::executor::block_on(store.run_concurrent(async |accessor| -> Result<_> {
            let mut streams = vec![];
            for _ in 0..2 {
                let mut round = vec![];
                for _ in 0..calls {
                    let (reader,) = func.call_concurrent(accessor, params.clone()).await?;
                    let items = Arc::new(Mutex::new(vec![]));
                    let (closed, on_close) = oneshot::channel();
                    let collect = Collect {
                        items: items.clone(),
                        per_read,
                        limit,
                        closed: Some(closed),
                    };
                    accessor.with(|mut store| reader.pipe(&mut store, collect))?;
                    round.push((items, on_close));
                }
                for (items, on_close) in round {
                    on_close.await?;
                    streams.push(std::mem::take(&mut *items.lock().unwrap()));
                }
            }
            Ok(streams)
        }))??;
    Ok(streams)
}

/// Collects a stream's items, signalling once the stream is closed, which
/// drops the consumer. The consumer closes the stream once `limit` items are
/// read.
struct Collect<T> {
    items: Arc<Mutex<Vec<T>>>,
    per_read: usize,
    limit: usize,
    closed: Option<oneshot::Sender<()>>,
}

impl<D, T: Lift + Send + Sync + 'static> StreamConsumer<D> for Collect<T> {
    type Item = T;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        mut store: StoreContextMut<D>,
        mut source: Source<'_, T>,
        _finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut items = self.items.lock().unwrap();
        let mut buffer = Vec::with_capacity(self.per_read.min(self.limit - items.len()));
        source.read(&mut store, &mut buffer)?;
        items.extend(buffer);
        Poll::Ready(Ok(match items.len() < self.limit {
            true => StreamResult::Completed,
            false => StreamResult::Dropped,
        }))
    }
}

impl<T> Drop for Collect<T> {
    fn drop(&mut self) {
        if let Some(closed) = self.closed.take() {
            let _ = closed.send(());
        }
    }
}

fn instantiate(
    bytes: &[u8],
    link: impl FnOnce(&mut Linker<()>) -> Result<()>,
) -> Result<(Store<()>, Instance)> {
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
    let instance = futures::executor::block_on(linker.instantiate_async(&mut store, &component))?;
    Ok((store, instance))
}

/// The named export, `interface` may be empty for world-level functions.
fn export(store: &mut Store<()>, instance: &Instance, interface: &str, name: &str) -> Func {
    let parent = match interface {
        "" => None,
        interface => Some(
            instance
                .get_export_index(&mut *store, None, interface)
                .expect("interface export"),
        ),
    };
    let index = instance
        .get_export_index(&mut *store, parent.as_ref(), name)
        .unwrap_or_else(|| panic!("missing export {name}"));
    instance.get_func(&mut *store, index).expect("func export")
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
