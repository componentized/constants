use anyhow::{Context, Result, bail};
use std::path::Path;
use wasm_encoder::{
    CodeSection, ConstExpr, CustomSection, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, Ieee32, Ieee64, Instruction, MemorySection, MemoryType, Module, TypeSection,
    ValType,
};
use wasm_metadata::Producers;
use wasm_wave::untyped::UntypedValue;
use wit_component::{ComponentEncoder, DecodedWasm, StringEncoding, metadata};
use wit_parser::abi::{AbiVariant, WasmType};
use wit_parser::{
    LiftLowerAbi, ManglingAndAbi, Resolve, WasmExport, WasmExportKind, WorldId, WorldItem, WorldKey,
};

use values::{Flat, Layout, optional_fields, parser_error};
use wasm_wave::ast::Node;

mod docs;
pub mod types;
mod values;

/// Values for [`Overrides::Value`], from `wasm_wave`.
///
/// Values are built with the [`WasmValue`](value::WasmValue) constructors,
/// e.g. [`Value::make_record`](value::WasmValue::make_record), from types
/// built with [`Type`](value::Type).
pub mod value {
    pub use wasm_wave::value::{Type, Value};
    pub use wasm_wave::wasm::{WasmValue, WasmValueError};
}

/// Address where constant data starts in linear memory. Kept non-zero so a
/// valid pointer is never null.
const DATA_BASE: u32 = 8;
const PAGE_SIZE: u64 = 65536;

/// Parses WIT from a file or directory and selects a world from it.
///
/// When `world` is `None`, the package must contain exactly one world.
pub fn load_world(path: impl AsRef<Path>, world: Option<&str>) -> Result<(Resolve, WorldId)> {
    let mut resolve = Resolve::default();
    let (pkg, _) = resolve.push_path(path)?;
    let world = resolve.select_world(&[pkg], world)?;
    Ok((resolve, world))
}

/// Parses WIT text and selects a world from it. Packages the world depends on
/// may be defined inline with nested `package ns:name { ... }` blocks.
///
/// When `world` is `None`, the package must contain exactly one world.
pub fn parse_world(text: &str, world: Option<&str>) -> Result<(Resolve, WorldId)> {
    let mut resolve = Resolve::default();
    let pkg = resolve.push_str("input.wit", text)?;
    let world = resolve.select_world(&[pkg], world)?;
    Ok((resolve, world))
}

/// Decodes a WIT package encoded as wasm and selects a world from it.
///
/// When `world` is `None`, the package must contain exactly one world.
pub fn decode_world(bytes: &[u8], world: Option<&str>) -> Result<(Resolve, WorldId)> {
    match wit_component::decode(bytes)? {
        DecodedWasm::WitPackage(resolve, pkg) => {
            let world = resolve.select_world(&[pkg], world)?;
            Ok((resolve, world))
        }
        DecodedWasm::Component(..) => bail!("expected a WIT package, found a component"),
    }
}

/// Creates a component that implements every export of `world` by returning a
/// constant value.
///
/// The world may only export functions (directly, or within interfaces) that
/// are synchronous and take no parameters. Their values may not reach
/// resources, handles, futures, streams, error contexts or maps, though the
/// result types may include them in branches the values don't take, e.g.
/// `none` for an `option<own<r>>`. Types the exports reference from other
/// interfaces become imports of the resulting component.
///
/// Each function's value is a WAVE expression following a `@value` tag in its
/// doc comment:
///
/// ```wit
/// /// The answer to everything.
/// /// @value 42
/// answer: func() -> u32;
/// ```
///
/// `overrides`, when provided, replaces values from the WIT, see [`Overrides`].
pub fn create_component(
    resolve: &Resolve,
    world: WorldId,
    overrides: Option<Overrides>,
) -> Result<Vec<u8>> {
    let module = create_module(resolve, world, overrides)?;
    ComponentEncoder::default()
        .validate(true)
        .module(&module)?
        .encode()
}

/// Values replacing those from the WIT.
///
/// The overrides are a record with a field per world export. A function
/// export's field holds its value, and an interface export's field (named by
/// the interface) holds a nested record with a field per function. Every field
/// is optional:
///
/// ```text
/// {
///   answer: 42,
///   constants: {
///     origin: {x: 0, y: 0},
///   },
/// }
/// ```
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Overrides<'a> {
    /// The record as WAVE source, `//` line comments are allowed. Source with
    /// nothing but whitespace and comments overrides nothing.
    Wave(&'a str),
    /// The record as a value, e.g. built with
    /// [`WasmValue::make_record`](value::WasmValue::make_record).
    /// The record's type only needs the fields being overridden.
    Value(&'a value::Value),
}

/// An exported function, the interface it belongs to, and its value.
struct Constant<'a> {
    interface: Option<&'a WorldKey>,
    func: &'a wit_parser::Function,
    src: &'a str,
    value: Value<'a>,
}

enum Value<'a> {
    Override(&'a Node),
    Doc(UntypedValue<'a>),
}

impl Constant<'_> {
    fn node(&self) -> &Node {
        match &self.value {
            Value::Override(node) => node,
            Value::Doc(value) => value.node(),
        }
    }
}

fn create_module(
    resolve: &Resolve,
    world: WorldId,
    overrides: Option<Overrides>,
) -> Result<Vec<u8>> {
    // every export must be a function returning a supported type
    let exports = &resolve.worlds[world].exports;
    let mut labels = vec![];
    for (key, item) in exports {
        let label = export_label(resolve, key)?;
        match item {
            WorldItem::Function(func) => types::check_function(func)?,
            WorldItem::Interface { id, .. } => {
                for func in resolve.interfaces[*id].functions.values() {
                    types::check_function(func).with_context(|| format!("interface `{label}`"))?;
                }
            }
            WorldItem::Type { .. } => bail!("worlds may not export types"),
        }
        if labels.contains(&label) {
            bail!("multiple exports are named `{label}`");
        }
        labels.push(label);
    }

    // match overrides to exports, values are written as WAVE to share its
    // parsing and type checking
    let written;
    let overrides_src = match overrides {
        None => "",
        Some(Overrides::Wave(src)) if is_blank(src) => "",
        Some(Overrides::Wave(src)) => src,
        Some(Overrides::Value(value)) => {
            written = wasm_wave::to_string(value).context("invalid overrides")?;
            &written
        }
    };
    let overrides = match overrides_src {
        "" => None,
        src => Some(
            UntypedValue::parse(src)
                .map_err(|e| parser_error(src, e))
                .context("invalid overrides")?,
        ),
    };
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let export_overrides = match &overrides {
        Some(value) => {
            optional_fields(overrides_src, value.node(), &labels).context("invalid overrides")?
        }
        None => vec![None; labels.len()],
    };

    // pair each exported function with its override or its `@value` doc
    let mut constants = vec![];
    for (((key, item), label), node) in exports.iter().zip(labels).zip(export_overrides) {
        let funcs = match item {
            WorldItem::Function(func) => vec![(None, func, node)],
            WorldItem::Interface { id, .. } => {
                let funcs = &resolve.interfaces[*id].functions;
                let names: Vec<&str> = funcs.keys().map(String::as_str).collect();
                let nodes = match node {
                    Some(node) => optional_fields(overrides_src, node, &names)
                        .with_context(|| format!("invalid overrides for interface `{label}`"))?,
                    None => vec![None; names.len()],
                };
                funcs
                    .values()
                    .zip(nodes)
                    .map(|(f, n)| (Some(key), f, n))
                    .collect()
            }
            WorldItem::Type { .. } => unreachable!(),
        };
        for (interface, func, node) in funcs {
            let name = qualified_name(resolve, interface, func);
            let location = resolve.source_map.render_location(func.span);
            let constant = match (node, docs::value(&func.docs)) {
                (Some(node), _) => Constant {
                    interface,
                    func,
                    src: overrides_src,
                    value: Value::Override(node),
                },
                (None, Some(src)) => Constant {
                    interface,
                    func,
                    src,
                    value: Value::Doc(
                        UntypedValue::parse(src)
                            .map_err(|e| parser_error(src, e))
                            .with_context(|| {
                                format!("invalid `{}` for `{name}` at {location}", docs::TAG)
                            })?,
                    ),
                },
                (None, None) => bail!(
                    "missing value for `{name}` at {location}: add a `{}` tag to its doc \
                     comment, or an override",
                    docs::TAG
                ),
            };
            constants.push(constant);
        }
    }

    // encode the values and generate a function returning each one
    let mut layout = Layout::new(resolve, DATA_BASE)?;
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut exports = ExportSection::new();

    for (index, constant) in constants.iter().enumerate() {
        let func = constant.func;
        let ty = func.result.as_ref().expect("checked by check_function");
        let sig = resolve.wasm_signature(AbiVariant::GuestExport, func);
        let node = constant.node();
        let context = || {
            let name = qualified_name(resolve, constant.interface, func);
            match constant.value {
                Value::Override(_) => format!("invalid override for `{name}`"),
                Value::Doc(_) => format!(
                    "invalid `{}` for `{name}` at {}",
                    docs::TAG,
                    resolve.source_map.render_location(func.span)
                ),
            }
        };
        layout.set_source(constant.src);

        let flat = if sig.retptr {
            vec![Flat::I32(
                layout.store_new(ty, node).with_context(context)? as i32
            )]
        } else {
            let mut flat = vec![];
            layout.flatten(ty, node, &mut flat).with_context(context)?;
            flat
        };

        let results: Vec<ValType> = sig.results.iter().map(val_type).collect();
        types.ty().function([], results);
        functions.function(index as u32);

        let mut body = Function::new([]);
        for value in flat {
            body.instruction(&match value {
                Flat::I32(v) => Instruction::I32Const(v),
                Flat::I64(v) => Instruction::I64Const(v),
                Flat::F32(v) => Instruction::F32Const(Ieee32::new(v)),
                Flat::F64(v) => Instruction::F64Const(Ieee64::new(v)),
            });
        }
        body.instruction(&Instruction::End);
        code.function(&body);

        let name = resolve.wasm_export_name(
            ManglingAndAbi::Legacy(LiftLowerAbi::Sync),
            WasmExport::Func {
                interface: constant.interface,
                func,
                kind: WasmExportKind::Normal,
            },
        );
        exports.export(&name, ExportKind::Func, index as u32);
    }

    let mut memories = MemorySection::new();
    let memory_size = (layout.base() as u64 + layout.data().len() as u64).max(1);
    memories.memory(MemoryType {
        minimum: memory_size.div_ceil(PAGE_SIZE),
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    exports.export("memory", ExportKind::Memory, 0);

    let mut data = DataSection::new();
    data.active(
        0,
        &ConstExpr::i32_const(layout.base() as i32),
        layout.data().iter().copied(),
    );

    let mut producers = Producers::default();
    producers.add(
        "processed-by",
        "componentized-constants",
        env!("CARGO_PKG_VERSION"),
    );
    let component_type = metadata::encode(resolve, world, StringEncoding::UTF8, Some(&producers))?;

    let mut module = Module::new();
    module.section(&types);
    module.section(&functions);
    module.section(&memories);
    module.section(&exports);
    module.section(&code);
    module.section(&data);
    module.section(&CustomSection {
        name: format!("component-type:{}", resolve.worlds[world].name).into(),
        data: component_type.into(),
    });
    Ok(module.finish())
}

/// Whether the WAVE source contains nothing but whitespace and comments.
fn is_blank(src: &str) -> bool {
    src.lines()
        .map(str::trim)
        .all(|line| line.is_empty() || line.starts_with("//"))
}

fn qualified_name(
    resolve: &Resolve,
    interface: Option<&WorldKey>,
    func: &wit_parser::Function,
) -> String {
    match interface {
        Some(key) => format!("{}#{}", resolve.name_world_key(key), func.name),
        None => func.name.clone(),
    }
}

/// The field name used for an export in the values record.
fn export_label(resolve: &Resolve, key: &WorldKey) -> Result<String> {
    Ok(match key {
        WorldKey::Name(name) => name.clone(),
        WorldKey::Interface(id) => resolve.interfaces[*id]
            .name
            .clone()
            .context("exported interfaces must be named")?,
    })
}

fn val_type(ty: &WasmType) -> ValType {
    match ty {
        WasmType::I32 | WasmType::Pointer | WasmType::Length => ValType::I32,
        WasmType::I64 | WasmType::PointerOrI64 => ValType::I64,
        WasmType::F32 => ValType::F32,
        WasmType::F64 => ValType::F64,
    }
}
