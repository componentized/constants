//! Runs `target/components/test-cli`, built by make, as `wasmtime run`
//! would, with WASIp3's async `wasi:cli/run`.

use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use test_harness::built_component;
use tokio::io::{AsyncRead, AsyncWrite};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{
    WasiCtx, WasiCtxView, WasiView,
    cli::{AsyncStdinStream, IsTerminal, StdinStream, StdoutStream},
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    p3::bindings::Command,
};

struct State {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

/// stdout or stderr, written straight to memory, so everything the command
/// wrote is there once it returns.
#[derive(Clone)]
struct Captured(MemoryOutputPipe);

impl IsTerminal for Captured {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for Captured {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.0.clone())
    }
}

/// stdin from memory, claiming to be a terminal.
struct Terminal(AsyncStdinStream);

impl IsTerminal for Terminal {
    fn is_terminal(&self) -> bool {
        true
    }
}

impl StdinStream for Terminal {
    fn async_stream(&self) -> Box<dyn AsyncRead + Send + Sync> {
        self.0.async_stream()
    }
}

/// What a run printed, and whether it succeeded.
struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// The engine and the command, compiled once for every test.
fn command() -> Result<&'static (Engine, Component)> {
    static COMMAND: OnceLock<Result<(Engine, Component), String>> = OnceLock::new();
    COMMAND
        .get_or_init(|| {
            let compile = || -> Result<_> {
                let mut config = Config::new();
                config.wasm_component_model_async(true);
                let engine = Engine::new(&config)?;
                let component = Component::from_file(&engine, built_component("test-cli")?)?;
                Ok((engine, component))
            };
            compile().map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

/// Runs the command with `args`, and `stdin` as its input.
async fn run(args: &[&str], stdin: &str) -> Result<Output> {
    run_with(args, stdin, false).await
}

/// Like [`run`], with stdin a terminal when `terminal`.
async fn run_with(args: &[&str], stdin: &str, terminal: bool) -> Result<Output> {
    let (engine, component) = command()?;
    let mut linker = Linker::<State>::new(engine);
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    let stdout = MemoryOutputPipe::new(1 << 20);
    let stderr = MemoryOutputPipe::new(1 << 20);
    let mut builder = WasiCtx::builder();
    builder
        .args(&[&["test-cli"], args].concat())
        .stdout(Captured(stdout.clone()))
        .stderr(Captured(stderr.clone()));
    let input = AsyncStdinStream::new(MemoryInputPipe::new(stdin.to_string()));
    match terminal {
        true => builder.stdin(Terminal(input)),
        false => builder.stdin(input),
    };
    let mut store = Store::new(
        engine,
        State {
            ctx: builder.build(),
            table: ResourceTable::default(),
        },
    );
    let command = Command::instantiate_async(&mut store, component, &linker).await?;
    let result = store
        .run_concurrent(async move |store| command.wasi_cli_run().call_run(store).await)
        .await??;
    Ok(Output {
        ok: result.is_ok(),
        stdout: String::from_utf8_lossy(&stdout.contents()).into(),
        stderr: String::from_utf8_lossy(&stderr.contents()).into(),
    })
}

fn lines(output: &str) -> Vec<&str> {
    output.lines().collect()
}

#[tokio::test(flavor = "current_thread")]
async fn it_prints_values() -> Result<()> {
    for (args, expected) in [
        (["math", "pi"], "3.141592653589793"),
        (["u8", "max"], "255"),
        (["s64", "min"], "-9223372036854775808"),
        (["f32", "epsilon"], "1.1920929e-7"),
    ] {
        let output = run(&args, "").await?;
        assert!(output.ok, "{args:?}: {}", output.stderr);
        assert_eq!(output.stdout.trim(), expected, "{args:?}");
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn it_reads_stream_items_as_stdin_asks() -> Result<()> {
    // the first item, then an empty line reads one item, a number that many
    let output = run(&["atomics", "incrementer", "10"], "\n\n3\n").await?;
    assert!(output.ok, "{}", output.stderr);
    assert_eq!(lines(&output.stdout), ["10", "11", "12", "13", "14", "15"]);
    let output = run(&["atomics", "incrementer", "10"], "").await?;
    assert_eq!(lines(&output.stdout), ["10"]);

    // the command ends with the stream
    let output = run(&["atomics", "decrementer", "2"], "\n\n\n\n").await?;
    assert_eq!(lines(&output.stdout), ["2", "1", "0"]);
    let output = run(&["math", "fibonacci"], "1000\n").await?;
    assert_eq!(output.stdout.lines().last(), Some("12200160415121876738"));

    // or when stdin closes
    let output = run(&["math", "odd"], "2\n").await?;
    assert_eq!(lines(&output.stdout), ["1", "3", "5"]);

    let output = run(&["random", "seeded-random-bytes", "42"], "8\n").await?;
    assert_eq!(
        lines(&output.stdout),
        ["149", "110", "235", "47", "38", "50", "215", "189", "3"]
    );

    let output = run(&["math", "even"], "x\n\n").await?;
    assert_eq!(lines(&output.stdout), ["0", "2"]);
    assert!(
        output.stderr.contains("expected an empty line or a number"),
        "{}",
        output.stderr
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn it_reports_errors() -> Result<()> {
    for (args, expected) in [
        (
            vec!["math", "nope"],
            "unknown function `nope` of interface `math`",
        ),
        (
            vec!["atomics", "incrementer", "abc"],
            "invalid argument `seed`",
        ),
        (
            vec!["atomics", "incrementer"],
            "expected 1 arguments, found 0",
        ),
        (vec![], "usage: test-cli <interface> <function>"),
    ] {
        let output = run(&args, "").await?;
        assert!(!output.ok, "{args:?}");
        assert!(
            output.stderr.contains(expected),
            "{args:?}: {}",
            output.stderr
        );
    }
    // the usage lists the functions
    let output = run(&[], "").await?;
    assert!(
        output
            .stderr
            .contains("  random seeded-random: async func(seed: u64) -> stream<u64>")
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn it_leaves_a_terminal_to_end_each_item_line() -> Result<()> {
    // a terminal echoes the newline ending each line of stdin, which ends the
    // line of the item printed for it, so the items printed for a line are
    // only separated by newlines, and the last item's line is ended on exit
    let output = run_with(&["atomics", "incrementer", "10"], "\n\n3\n", true).await?;
    assert!(output.ok, "{}", output.stderr);
    assert_eq!(output.stdout, "10111213\n14\n15\n");
    // the line asking for an item after the stream's last ends its line
    let output = run_with(&["atomics", "decrementer", "1"], "\n\n", true).await?;
    assert_eq!(output.stdout, "10");
    let output = run_with(&["atomics", "decrementer", "1"], "2\n", true).await?;
    assert_eq!(output.stdout, "10\n");
    Ok(())
}
