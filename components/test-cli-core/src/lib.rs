//! A command calling the functions of the constants data interfaces, for
//! testing:
//!
//! ```sh
//! test-cli <interface> <function> [arguments...]
//! ```
//!
//! The arguments are parsed as the function's parameters, and its result is
//! printed. A stream's first item is printed, then items are read as stdin's
//! lines ask for them: an empty line reads the next item, a number that many.
//! The command ends when the stream ends or stdin closes.
//!
//! The command exports WASIp3's async `wasi:cli/run`, so it can wait for a
//! stream's items, which a synchronous entry point can't. Its arguments, stdin,
//! stdout and stderr are WASIp3's too, stdin read from a stream, and stdout and
//! stderr written to streams.

use std::{fmt::Debug, str::FromStr};
use wit_bindgen::{FutureReader, StreamReader, StreamResult, StreamWriter};

use crate::wasi::cli::{environment, stderr, stdin, stdout, terminal_stdin, types::ErrorCode};

wit_bindgen::generate!({
    path: "../wit",
    world: "test-cli-core",
    generate_all
});

include!(concat!(env!("OUT_DIR"), "/dispatch.rs"));

struct Cli;

impl exports::wasi::cli::run::Guest for Cli {
    async fn run() -> Result<(), ()> {
        let args: Vec<String> = environment::get_arguments().into_iter().skip(1).collect();
        let mut io = Io::new();
        let result = match args.as_slice() {
            [interface, function, args @ ..] => dispatch(&mut io, interface, function, args).await,
            _ => Err(usage()),
        };
        if let Err(message) = &result {
            io.stderr.write(format!("{message}\n")).await;
        }
        io.close().await;
        result.map_err(|_| ())
    }
}

export!(Cli);

/// The command's stdin, stdout and stderr.
pub struct Io {
    stdin: Lines,
    stdout: Output,
    stderr: Output,
    /// Whether stdin is a terminal, which echoes the newline each line ends
    /// with.
    terminal: bool,
    /// Whether stdout's last line is a stream item not yet ended by a newline.
    unended: bool,
}

impl Io {
    fn new() -> Self {
        let (stdin, _) = stdin::read_via_stream();
        Self {
            stdin: Lines {
                stream: stdin,
                buffer: vec![],
                closed: false,
            },
            stdout: Output::new(stdout::write_via_stream),
            stderr: Output::new(stderr::write_via_stream),
            terminal: terminal_stdin::get_terminal_stdin().is_some(),
            unended: false,
        }
    }

    /// Prints a stream's item. A terminal's echo of the newline asking for the
    /// next item ends the item's line, so the item isn't followed by a blank
    /// line, a newline only separates items printed for the same line.
    async fn print_item(&mut self, item: String) {
        if !self.terminal {
            return self.print(item).await;
        }
        if self.unended {
            self.stdout.write("\n".into()).await;
        }
        self.stdout.write(item).await;
        self.unended = true;
    }

    /// The next line of stdin, whose echo ends any item printed last.
    async fn next_line(&mut self) -> Option<String> {
        let line = self.stdin.next().await;
        if line.is_some() {
            self.unended = false;
        }
        line
    }

    async fn print(&mut self, line: String) {
        self.stdout.write(line + "\n").await;
    }

    async fn error(&mut self, line: String) {
        self.stderr.write(line + "\n").await;
    }

    /// Closes stdout and stderr, once everything written has been taken,
    /// ending the last item's line.
    async fn close(mut self) {
        if self.unended {
            self.stdout.write("\n".into()).await;
        }
        self.stdout.close().await;
        self.stderr.close().await;
    }
}

/// Bytes written to stdout or stderr through a stream.
struct Output {
    stream: StreamWriter<u8>,
    done: FutureReader<Result<(), ErrorCode>>,
}

impl Output {
    fn new(write_via_stream: fn(StreamReader<u8>) -> FutureReader<Result<(), ErrorCode>>) -> Self {
        let (stream, reader) = wit_stream::new();
        Self {
            stream,
            done: write_via_stream(reader),
        }
    }

    async fn write(&mut self, text: String) {
        self.stream.write_all(text.into_bytes()).await;
    }

    async fn close(self) {
        drop(self.stream);
        let _ = self.done.await;
    }
}

/// stdin's lines, read from its stream.
struct Lines {
    stream: StreamReader<u8>,
    buffer: Vec<u8>,
    closed: bool,
}

impl Lines {
    /// The next line, without its newline, or `None` once stdin closes.
    async fn next(&mut self) -> Option<String> {
        loop {
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=end).collect();
                return Some(String::from_utf8_lossy(&line[..end]).into());
            }
            if self.closed {
                return match self.buffer.is_empty() {
                    true => None,
                    false => {
                        Some(String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into())
                    }
                };
            }
            let (result, bytes) = self.stream.read(Vec::with_capacity(4096)).await;
            self.buffer.extend(bytes);
            if !matches!(result, StreamResult::Complete(_)) {
                self.closed = true;
            }
        }
    }
}

fn usage() -> String {
    let mut usage = String::from(
        "usage: test-cli <interface> <function> [arguments...]\n\n\
         A stream's first item is printed, then an empty line on stdin reads the next \
         item, a number reads that many.\n\nfunctions:\n",
    );
    for (interface, function, signature) in FUNCTIONS {
        usage.push_str(&format!("  {interface} {function}: {signature}\n"));
    }
    usage
}

fn expect_args(args: &[String], count: usize) -> Result<(), String> {
    match args.len() == count {
        true => Ok(()),
        false => Err(format!("expected {count} arguments, found {}", args.len())),
    }
}

fn parse<T: FromStr>(arg: &str, name: &str) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    arg.parse()
        .map_err(|e| format!("invalid argument `{name}`, {arg:?}: {e}"))
}

async fn print_value(io: &mut Io, value: &impl Debug) {
    io.print(format!("{value:?}")).await;
}

/// Prints the stream's first item, then items as stdin's lines ask for them,
/// until the stream ends or stdin closes, which closes the stream.
async fn read_stream<T: Debug + 'static>(io: &mut Io, mut stream: StreamReader<T>) {
    match stream.next().await {
        Some(item) => io.print_item(format!("{item:?}")).await,
        None => return,
    }
    while let Some(line) = io.next_line().await {
        let count = match line.trim() {
            "" => 1,
            count => match count.parse::<usize>() {
                Ok(count) => count,
                Err(_) => {
                    io.error(format!(
                        "expected an empty line or a number of items, found {count:?}"
                    ))
                    .await;
                    continue;
                }
            },
        };
        for _ in 0..count {
            match stream.next().await {
                Some(item) => io.print_item(format!("{item:?}")).await,
                None => return,
            }
        }
    }
}
