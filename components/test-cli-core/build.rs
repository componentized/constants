//! Generates the cli's dispatch from the test-cli-core world, a match arm calling
//! each function of each imported interface, so the cli follows the WIT.

use std::{env, fmt::Write, fs, path::PathBuf};
use wit_parser::{Function, Resolve, Type, TypeDefKind, WorldItem};

fn main() {
    let wit = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../wit");
    println!("cargo::rerun-if-changed={}", wit.display());
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve
        .push_path(&wit)
        .expect("components/wit, run `make wit`");
    let world = resolve
        .select_world(&[pkg], Some("test-cli-core"))
        .expect("test-cli-core world");

    let mut arms = String::new();
    let mut functions = String::new();
    for (key, item) in &resolve.worlds[world].imports {
        let WorldItem::Interface { id, .. } = item else {
            continue;
        };
        let interface = &resolve.interfaces[*id];
        let Some(name) = &interface.name else {
            continue;
        };
        let package = &resolve.packages[interface.package.expect("a package")].name;
        // only the constants interfaces, not WASI's
        if (package.namespace.as_str(), package.name.as_str()) != ("componentized", "constants") {
            continue;
        }
        let module = format!(
            "{}::{}::{}",
            ident(&package.namespace),
            ident(&package.name),
            ident(name)
        );
        let _ = key;
        for func in interface.functions.values() {
            let signature = signature(&resolve, func);
            writeln!(functions, "    ({name:?}, {:?}, {signature:?}),", func.name).unwrap();
            writeln!(arms, "        ({name:?}, {:?}) => {{", func.name).unwrap();
            writeln!(
                arms,
                "            expect_args(args, {})?;",
                func.params.len()
            )
            .unwrap();
            let mut call_args = vec![];
            let mut parsable = true;
            for (index, param) in func.params.iter().enumerate() {
                match rust_type(&resolve, &param.ty) {
                    Some(ty) => {
                        writeln!(
                            arms,
                            "            let a{index}: {ty} = parse(&args[{index}], {:?})?;",
                            param.name
                        )
                        .unwrap();
                        call_args.push(format!("a{index}"));
                    }
                    None => parsable = false,
                }
            }
            if !parsable {
                writeln!(
                    arms,
                    "            Err(format!(\"`{}` takes parameters the cli can't parse\"))\n        }}",
                    func.name
                )
                .unwrap();
                continue;
            }
            let mut call = format!("{module}::{}({})", ident(&func.name), call_args.join(", "));
            if func.kind.is_async() {
                call = format!("{call}.await");
            }
            let stream = func
                .result
                .as_ref()
                .is_some_and(|ty| matches!(kind(&resolve, ty), Some(TypeDefKind::Stream(_))));
            match (stream, &func.result) {
                (true, _) => writeln!(arms, "            read_stream(io, {call}).await;").unwrap(),
                (false, Some(_)) => {
                    writeln!(arms, "            print_value(io, &{call}).await;").unwrap()
                }
                (false, None) => writeln!(arms, "            {call};").unwrap(),
            }
            writeln!(arms, "            Ok(())\n        }}").unwrap();
        }
    }

    let code = format!(
        "/// Each function: its interface, its name and its signature.\n\
         pub const FUNCTIONS: &[(&str, &str, &str)] = &[\n{functions}];\n\n\
         /// Calls `function` of `interface` with `args` parsed as its parameters.\n\
         pub async fn dispatch(io: &mut Io, interface: &str, function: &str, args: &[String]) -> Result<(), String> {{\n    \
         match (interface, function) {{\n{arms}        \
         _ => Err(format!(\"unknown function `{{function}}` of interface `{{interface}}`\")),\n    \
         }}\n}}\n"
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("dispatch.rs");
    fs::write(out, code).unwrap();
}

/// A WIT name as wit-bindgen names it in Rust.
fn ident(name: &str) -> String {
    let name = name.replace('-', "_");
    match name.as_str() {
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self" | "static"
        | "struct" | "super" | "trait" | "true" | "type" | "unsafe" | "use" | "where" | "while" => {
            format!("{name}_")
        }
        _ => name,
    }
}

fn kind<'a>(resolve: &'a Resolve, ty: &Type) -> Option<&'a TypeDefKind> {
    match ty {
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::Type(ty) => kind(resolve, ty),
            kind => Some(kind),
        },
        _ => None,
    }
}

/// The Rust type a parameter is parsed into, for the types the cli parses.
fn rust_type(resolve: &Resolve, ty: &Type) -> Option<&'static str> {
    Some(match ty {
        Type::Bool => "bool",
        Type::U8 => "u8",
        Type::U16 => "u16",
        Type::U32 => "u32",
        Type::U64 => "u64",
        Type::S8 => "i8",
        Type::S16 => "i16",
        Type::S32 => "i32",
        Type::S64 => "i64",
        Type::F32 => "f32",
        Type::F64 => "f64",
        Type::Char => "char",
        Type::String => "String",
        Type::Id(id) => match &resolve.types[*id].kind {
            TypeDefKind::Type(ty) => return rust_type(resolve, ty),
            _ => return None,
        },
        Type::ErrorContext => return None,
    })
}

/// The function's WIT signature, for the usage.
fn signature(resolve: &Resolve, func: &Function) -> String {
    let params: Vec<String> = func
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, type_name(resolve, &p.ty)))
        .collect();
    let prefix = if func.kind.is_async() {
        "async func"
    } else {
        "func"
    };
    match &func.result {
        Some(ty) => format!(
            "{prefix}({}) -> {}",
            params.join(", "),
            type_name(resolve, ty)
        ),
        None => format!("{prefix}({})", params.join(", ")),
    }
}

fn type_name(resolve: &Resolve, ty: &Type) -> String {
    match ty {
        Type::Id(id) => {
            let def = &resolve.types[*id];
            match (&def.name, &def.kind) {
                (Some(name), _) => name.clone(),
                (None, TypeDefKind::Stream(Some(ty))) => {
                    format!("stream<{}>", type_name(resolve, ty))
                }
                (None, TypeDefKind::List(ty)) => format!("list<{}>", type_name(resolve, ty)),
                (None, TypeDefKind::Option(ty)) => format!("option<{}>", type_name(resolve, ty)),
                (None, TypeDefKind::Tuple(t)) => format!(
                    "tuple<{}>",
                    t.types
                        .iter()
                        .map(|ty| type_name(resolve, ty))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                (None, kind) => kind.as_str().to_string(),
            }
        }
        Type::Bool => "bool".into(),
        Type::U8 => "u8".into(),
        Type::U16 => "u16".into(),
        Type::U32 => "u32".into(),
        Type::U64 => "u64".into(),
        Type::S8 => "s8".into(),
        Type::S16 => "s16".into(),
        Type::S32 => "s32".into(),
        Type::S64 => "s64".into(),
        Type::F32 => "f32".into(),
        Type::F64 => "f64".into(),
        Type::Char => "char".into(),
        Type::String => "string".into(),
        Type::ErrorContext => "error-context".into(),
    }
}
