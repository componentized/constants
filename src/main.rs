use anyhow::{Context, Result};
use clap::Parser;
use componentized_constants::{Overrides, create_component, load_world};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};

fn main() {
    if let Err(message) = Command::parse().exec() {
        eprintln!("{message:#}");
        std::process::exit(1);
    }
}

#[derive(Debug, Parser)]
#[command()]
/// Create a wasm component whose exported functions return constant values.
///
/// Every function exported by the world must be synchronous, accept no
/// parameters and return a value. Values can't reach maps, handles, futures,
/// streams or error contexts, though result types may include them in
/// branches the value doesn't take, e.g. `none` for an `option<own<r>>`.
///
/// Each function's value is a WAVE expression following a `@value` tag in its
/// doc comment, e.g. `/// @value 42`.
struct Command {
    /// Output to a file, '-' for stdout
    #[arg(short('o'), long("output"), name("wasm-file"))]
    output: String,
    /// WIT file or directory defining the world to implement
    #[arg(short('w'), long("wit"), name("wit-path"))]
    wit: String,
    /// Name of the world to implement, required if the WIT package defines
    /// more than one world
    #[arg(long("world"), name("world"))]
    world: Option<String>,
    /// WAVE encoded overrides file, '-' for stdin. The file contains a
    /// record with an optional field for each export of the world, replacing
    /// the value from the `@value` doc tag.
    #[arg(short('f'), long("overrides"), name("overrides-file"))]
    overrides: Option<String>,
}

impl Command {
    fn exec(&self) -> Result<()> {
        let (resolve, world) = load_world(&self.wit, self.world.as_deref())?;

        let mut overrides = String::new();
        match self.overrides.as_deref() {
            None => {}
            Some("-") => {
                eprintln!("Reading overrides from stdin (ctrl-d to finish)");
                io::stdin().read_to_string(&mut overrides)?;
            }
            Some(path) => {
                eprintln!("Reading overrides from {path}");
                File::open(Path::new(path))
                    .and_then(|mut file| file.read_to_string(&mut overrides))
                    .with_context(|| format!("unable to read {path}"))?;
            }
        };

        let component = create_component(&resolve, world, Some(Overrides::Wave(&overrides)))?;

        let mut output = match self.output.as_str() {
            "-" => {
                eprintln!("Writing component to stdout");
                Box::new(io::stdout()) as Box<dyn Write>
            }
            path => {
                eprintln!("Writing component to {path}");
                Box::new(File::create(Path::new(path))?) as Box<dyn Write>
            }
        };
        output.write_all(&component)?;
        Ok(())
    }
}
