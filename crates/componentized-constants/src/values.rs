use anyhow::{Error, Result, anyhow, bail};
use std::ops::Range;
use wasm_wave::ast::{Node, NodeType};
use wasm_wave::parser::ParserError;
use wit_parser::abi::{FlatTypes, WasmType};
use wit_parser::{Int, Resolve, SizeAlign, Type, TypeDefKind};

use crate::types::{dealias, describe};

/// A core wasm value produced by flattening a component value.
#[derive(Debug, Clone, Copy)]
pub enum Flat {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

/// Lays out constant values in linear memory following the canonical ABI.
///
/// Values are parsed from WAVE nodes and checked against their WIT type as
/// they are encoded. Everything is written into a single data segment that
/// is loaded at `base`. Values may come from different WAVE sources; see
/// [`Layout::set_source`].
pub struct Layout<'a> {
    resolve: &'a Resolve,
    sizes: SizeAlign,
    src: &'a str,
    base: u32,
    data: Vec<u8>,
}

impl Flat {
    /// Converts the value to a joined variant flat type, following the
    /// canonical ABI's `lower_flat_variant`.
    fn coerce(self, want: &WasmType) -> Flat {
        match (self, want) {
            (Flat::F32(bits), WasmType::I32 | WasmType::Pointer | WasmType::Length) => {
                Flat::I32(bits as i32)
            }
            (Flat::I32(v), WasmType::I64 | WasmType::PointerOrI64) => Flat::I64(v as u32 as i64),
            (Flat::F32(bits), WasmType::I64 | WasmType::PointerOrI64) => Flat::I64(bits as i64),
            (Flat::F64(bits), WasmType::I64 | WasmType::PointerOrI64) => Flat::I64(bits as i64),
            (value, _) => value,
        }
    }

    fn zero(ty: &WasmType) -> Flat {
        match ty {
            WasmType::I32 | WasmType::Pointer | WasmType::Length => Flat::I32(0),
            WasmType::I64 | WasmType::PointerOrI64 => Flat::I64(0),
            WasmType::F32 => Flat::F32(0),
            WasmType::F64 => Flat::F64(0),
        }
    }
}

impl<'a> Layout<'a> {
    pub fn new(resolve: &'a Resolve, base: u32) -> Result<Self> {
        let mut sizes = SizeAlign::default();
        sizes.fill(resolve)?;
        Ok(Self {
            resolve,
            sizes,
            src: "",
            base,
            data: Vec::new(),
        })
    }

    /// Sets the WAVE source text that subsequently encoded nodes were parsed
    /// from.
    pub fn set_source(&mut self, src: &'a str) {
        self.src = src;
    }

    pub fn base(&self) -> u32 {
        self.base
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Reserves `size` bytes aligned to `align`, returning the address.
    fn alloc(&mut self, size: usize, align: usize) -> Result<u32> {
        let base = self.base as usize;
        let start = (base + self.data.len()).next_multiple_of(align);
        let end = start + size;
        if end > u32::MAX as usize {
            bail!("constant values exceed the 32-bit address space");
        }
        self.data.resize(end - base, 0);
        Ok(start as u32)
    }

    fn write(&mut self, addr: u32, bytes: &[u8]) {
        let offset = (addr - self.base) as usize;
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    fn size(&self, ty: &Type) -> usize {
        self.sizes.size(ty).size_wasm32()
    }

    fn align(&self, ty: &Type) -> usize {
        self.sizes.align(ty).align_wasm32()
    }

    /// Allocates space for `ty`, stores the value there and returns the
    /// address.
    pub fn store_new(&mut self, ty: &Type, node: &Node) -> Result<u32> {
        let addr = self.alloc(self.size(ty), self.align(ty))?;
        self.store(ty, node, addr)?;
        Ok(addr)
    }

    /// Stores the value into memory at `addr`, which must already be
    /// allocated with the size and alignment of `ty`.
    pub fn store(&mut self, ty: &Type, node: &Node, addr: u32) -> Result<()> {
        match dealias(self.resolve, *ty) {
            Type::Bool => self.write(addr, &[self.parse(node.as_bool())? as u8]),
            Type::U8 => self.write(addr, &self.number::<u8>(node)?.to_le_bytes()),
            Type::S8 => self.write(addr, &self.number::<i8>(node)?.to_le_bytes()),
            Type::U16 => self.write(addr, &self.number::<u16>(node)?.to_le_bytes()),
            Type::S16 => self.write(addr, &self.number::<i16>(node)?.to_le_bytes()),
            Type::U32 => self.write(addr, &self.number::<u32>(node)?.to_le_bytes()),
            Type::S32 => self.write(addr, &self.number::<i32>(node)?.to_le_bytes()),
            Type::U64 => self.write(addr, &self.number::<u64>(node)?.to_le_bytes()),
            Type::S64 => self.write(addr, &self.number::<i64>(node)?.to_le_bytes()),
            Type::F32 => self.write(addr, &self.number::<f32>(node)?.to_le_bytes()),
            Type::F64 => self.write(addr, &self.number::<f64>(node)?.to_le_bytes()),
            Type::Char => self.write(addr, &(self.char(node)? as u32).to_le_bytes()),
            Type::String => {
                let (ptr, len) = self.string(node)?;
                self.write(addr, &ptr.to_le_bytes());
                self.write(addr + 4, &len.to_le_bytes());
            }
            Type::ErrorContext => return Err(self.unsupported(ty, node)),
            Type::Id(id) => match &self.resolve.types[id].kind {
                TypeDefKind::Record(r) => {
                    let types: Vec<Type> = r.fields.iter().map(|f| f.ty).collect();
                    let values = self.record(node, r)?;
                    let offsets = self.sizes.field_offsets(types.iter());
                    for ((offset, ty), value) in offsets.into_iter().zip(values) {
                        // omitted option fields are left zeroed, which is `none`
                        if let Some(value) = value {
                            self.store(ty, value, addr + offset.size_wasm32() as u32)?;
                        }
                    }
                }
                TypeDefKind::Tuple(t) => {
                    let values = self.tuple(node, t.types.len())?;
                    self.store_fields(&t.types, &values, addr)?;
                }
                TypeDefKind::Flags(f) => {
                    let bits = self.flags(node, f)?;
                    let bytes = bits.to_le_bytes();
                    self.write(addr, &bytes[..self.size(ty)]);
                }
                TypeDefKind::List(elem) => {
                    let (ptr, len) = self.list(elem, node)?;
                    self.write(addr, &ptr.to_le_bytes());
                    self.write(addr + 4, &len.to_le_bytes());
                }
                TypeDefKind::FixedLengthList(elem, n) => {
                    let values = self.fixed_list(node, *n)?;
                    let stride = self.size(elem) as u32;
                    for (i, value) in values.iter().enumerate() {
                        self.store(elem, value, addr + i as u32 * stride)?;
                    }
                }
                TypeDefKind::Enum(e) => {
                    let index = self.enum_case(node, e)?;
                    self.write_tag(e.tag(), index, addr);
                }
                TypeDefKind::Option(t) => match self.option(node, t)? {
                    None => self.write_tag(Int::U8, 0, addr),
                    Some(payload) => {
                        self.write_tag(Int::U8, 1, addr);
                        let offset = self.sizes.payload_offset(Int::U8, [None, Some(t)]);
                        self.store(t, payload, addr + offset.size_wasm32() as u32)?;
                    }
                },
                TypeDefKind::Result(r) => {
                    let (index, payload) = self.result_case(node, r)?;
                    self.write_tag(Int::U8, index, addr);
                    let cases = [r.ok.as_ref(), r.err.as_ref()];
                    if let (Some(ty), Some(payload)) = (cases[index], payload) {
                        let offset = self.sizes.payload_offset(Int::U8, cases);
                        self.store(ty, payload, addr + offset.size_wasm32() as u32)?;
                    }
                }
                TypeDefKind::Variant(v) => {
                    let (index, payload) = self.variant_case(node, v)?;
                    self.write_tag(v.tag(), index, addr);
                    if let (Some(ty), Some(payload)) = (&v.cases[index].ty, payload) {
                        let cases = v.cases.iter().map(|c| c.ty.as_ref());
                        let offset = self.sizes.payload_offset(v.tag(), cases).size_wasm32();
                        self.store(ty, payload, addr + offset as u32)?;
                    }
                }
                // TODO add map support once WAVE defines a syntax for maps,
                // they're currently `unsupported`. Maps are stored like a
                // `list<tuple<k, v>>`, and duplicate keys should be rejected.
                TypeDefKind::Map(..)
                | TypeDefKind::Resource
                | TypeDefKind::Handle(_)
                | TypeDefKind::Future(_)
                | TypeDefKind::Stream(_)
                | TypeDefKind::Type(_)
                | TypeDefKind::Unknown => return Err(self.unsupported(ty, node)),
            },
        }
        Ok(())
    }

    fn store_fields(&mut self, types: &[Type], values: &[&Node], addr: u32) -> Result<()> {
        let offsets = self.sizes.field_offsets(types.iter());
        for ((offset, ty), value) in offsets.into_iter().zip(values) {
            self.store(ty, value, addr + offset.size_wasm32() as u32)?;
        }
        Ok(())
    }

    /// Flattens the value into core wasm values, as returned directly from a
    /// function whose result does not need a return pointer.
    pub fn flatten(&mut self, ty: &Type, node: &Node, out: &mut Vec<Flat>) -> Result<()> {
        match dealias(self.resolve, *ty) {
            Type::Bool => out.push(Flat::I32(self.parse(node.as_bool())? as i32)),
            Type::U8 => out.push(Flat::I32(self.number::<u8>(node)? as i32)),
            Type::S8 => out.push(Flat::I32(self.number::<i8>(node)? as i32)),
            Type::U16 => out.push(Flat::I32(self.number::<u16>(node)? as i32)),
            Type::S16 => out.push(Flat::I32(self.number::<i16>(node)? as i32)),
            Type::U32 => out.push(Flat::I32(self.number::<u32>(node)? as i32)),
            Type::S32 => out.push(Flat::I32(self.number::<i32>(node)?)),
            Type::U64 => out.push(Flat::I64(self.number::<u64>(node)? as i64)),
            Type::S64 => out.push(Flat::I64(self.number::<i64>(node)?)),
            Type::F32 => out.push(Flat::F32(self.number::<f32>(node)?.to_bits())),
            Type::F64 => out.push(Flat::F64(self.number::<f64>(node)?.to_bits())),
            Type::Char => out.push(Flat::I32(self.char(node)? as i32)),
            Type::String => {
                let (ptr, len) = self.string(node)?;
                out.extend([Flat::I32(ptr as i32), Flat::I32(len as i32)]);
            }
            Type::ErrorContext => return Err(self.unsupported(ty, node)),
            Type::Id(id) => match &self.resolve.types[id].kind {
                TypeDefKind::Record(r) => {
                    let values = self.record(node, r)?;
                    for (field, value) in r.fields.iter().zip(values) {
                        match value {
                            Some(value) => self.flatten(&field.ty, value, out)?,
                            // omitted option fields are `none`, which flattens to zeros
                            None => out.extend(self.flat_types(&field.ty)?.iter().map(Flat::zero)),
                        }
                    }
                }
                TypeDefKind::Tuple(t) => {
                    let values = self.tuple(node, t.types.len())?;
                    for (ty, value) in t.types.iter().zip(values) {
                        self.flatten(ty, value, out)?;
                    }
                }
                TypeDefKind::Flags(f) => out.push(Flat::I32(self.flags(node, f)? as i32)),
                TypeDefKind::List(elem) => {
                    let (ptr, len) = self.list(elem, node)?;
                    out.extend([Flat::I32(ptr as i32), Flat::I32(len as i32)]);
                }
                // TODO restore map support once WAVE defines a syntax for maps,
                // they're currently `unsupported`. Maps flatten like a list, to a
                // pointer and length.
                TypeDefKind::FixedLengthList(elem, n) => {
                    for value in self.fixed_list(node, *n)? {
                        self.flatten(elem, value, out)?;
                    }
                }
                TypeDefKind::Enum(e) => out.push(Flat::I32(self.enum_case(node, e)? as i32)),
                TypeDefKind::Option(t) => {
                    let payload = self.option(node, t)?;
                    let case = (payload.is_some() as usize, payload.map(|p| (t, p)));
                    self.flatten_case(&Type::Id(id), case, out)?;
                }
                TypeDefKind::Result(r) => {
                    let (index, payload) = self.result_case(node, r)?;
                    let payload = [r.ok.as_ref(), r.err.as_ref()][index].zip(payload);
                    self.flatten_case(&Type::Id(id), (index, payload), out)?;
                }
                TypeDefKind::Variant(v) => {
                    let (index, payload) = self.variant_case(node, v)?;
                    let payload = v.cases[index].ty.as_ref().zip(payload);
                    self.flatten_case(&Type::Id(id), (index, payload), out)?;
                }
                TypeDefKind::Map(..)
                | TypeDefKind::Resource
                | TypeDefKind::Handle(_)
                | TypeDefKind::Future(_)
                | TypeDefKind::Stream(_)
                | TypeDefKind::Type(_)
                | TypeDefKind::Unknown => return Err(self.unsupported(ty, node)),
            },
        }
        Ok(())
    }

    /// Flattens a variant-like value: the case index followed by the payload,
    /// widened to fit the flat types joined across every case, with unused
    /// trailing slots zeroed.
    fn flatten_case(
        &mut self,
        ty: &Type,
        (index, payload): (usize, Option<(&Type, &Node)>),
        out: &mut Vec<Flat>,
    ) -> Result<()> {
        let mut case = vec![];
        if let Some((ty, payload)) = payload {
            self.flatten(ty, payload, &mut case)?;
        }
        let joined = self.flat_types(ty)?;
        out.push(Flat::I32(index as i32));
        for (i, want) in joined[1..].iter().enumerate() {
            out.push(match case.get(i) {
                Some(value) => value.coerce(want),
                None => Flat::zero(want),
            });
        }
        Ok(())
    }

    /// Values can't be expressed for resources, handles, futures, streams,
    /// error contexts or maps. Types may still contain them, so long as a
    /// value doesn't reach them, e.g. `none` for an `option<own<r>>`, or `[]`
    /// for a `list<future<u8>>`.
    fn unsupported(&self, ty: &Type, node: &Node) -> Error {
        // TODO restore map support once WAVE defines a syntax for maps, see the
        // `TypeDefKind::Map` TODOs in `store` and `flatten`
        let ty = dealias(self.resolve, *ty);
        self.error(
            node,
            &format!("values of {} are not supported", describe(self.resolve, ty)),
        )
    }

    fn flat_types(&self, ty: &Type) -> Result<Vec<WasmType>> {
        let mut storage = [WasmType::I32; 64];
        let mut flat = FlatTypes::new(&mut storage);
        if !self.resolve.push_flat(ty, &mut flat) {
            bail!("too many flat values");
        }
        Ok(flat.to_vec())
    }

    fn write_tag(&mut self, tag: Int, index: usize, addr: u32) {
        let bytes = (index as u32).to_le_bytes();
        let len = match tag {
            Int::U8 => 1,
            Int::U16 => 2,
            Int::U32 => 4,
            Int::U64 => unreachable!("discriminants are at most 32 bits"),
        };
        self.write(addr, &bytes[..len]);
    }

    fn enum_case(&self, node: &Node, e: &wit_parser::Enum) -> Result<usize> {
        let label = self.parse(node.as_enum(self.src))?;
        e.cases
            .iter()
            .position(|c| c.name == label)
            .ok_or_else(|| self.error(node, &format!("unknown enum case `{label}`")))
    }

    /// Resolves the variant case and its payload, which must be present exactly
    /// when the case defines a payload type.
    fn variant_case<'n>(
        &self,
        node: &'n Node,
        v: &wit_parser::Variant,
    ) -> Result<(usize, Option<&'n Node>)> {
        let (label, payload) = self.parse(node.as_variant(self.src))?;
        let Some(index) = v.cases.iter().position(|c| c.name == label) else {
            return Err(self.error(node, &format!("unknown variant case `{label}`")));
        };
        match (&v.cases[index].ty, payload) {
            (Some(_), None) => Err(self.error(node, &format!("case `{label}` requires a payload"))),
            (None, Some(_)) => Err(self.error(node, &format!("case `{label}` has no payload"))),
            _ => Ok((index, payload)),
        }
    }

    fn string(&mut self, node: &Node) -> Result<(u32, u32)> {
        let value = self.parse(node.as_str(self.src))?;
        let bytes = value.as_bytes();
        let ptr = self.alloc(bytes.len(), 1)?;
        self.write(ptr, bytes);
        Ok((ptr, bytes.len() as u32))
    }

    fn list(&mut self, elem: &Type, node: &Node) -> Result<(u32, u32)> {
        let values: Vec<&Node> = self.parse(node.as_list())?.collect();
        let stride = self.size(elem);
        let ptr = self.alloc(stride * values.len(), self.align(elem))?;
        for (i, value) in values.iter().enumerate() {
            self.store(elem, value, ptr + (i * stride) as u32)?;
        }
        Ok((ptr, values.len() as u32))
    }

    fn fixed_list<'n>(&self, node: &'n Node, n: u32) -> Result<Vec<&'n Node>> {
        let values: Vec<&Node> = self.parse(node.as_list())?.collect();
        if values.len() != n as usize {
            return Err(self.error(
                node,
                &format!("expected {n} elements, found {}", values.len()),
            ));
        }
        Ok(values)
    }

    fn tuple<'n>(&self, node: &'n Node, n: usize) -> Result<Vec<&'n Node>> {
        let values: Vec<&Node> = self.parse(node.as_tuple())?.collect();
        if values.len() != n {
            return Err(self.error(
                node,
                &format!("expected a tuple of {n} values, found {}", values.len()),
            ));
        }
        Ok(values)
    }

    /// Matches record fields by name, in any order. Fields may only be
    /// omitted when their type is an option.
    fn record<'n>(&self, node: &'n Node, r: &wit_parser::Record) -> Result<Vec<Option<&'n Node>>> {
        let names: Vec<&str> = r.fields.iter().map(|f| f.name.as_str()).collect();
        let optional: Vec<bool> = r.fields.iter().map(|f| self.is_option(&f.ty)).collect();
        match_fields(self.src, node, &names, &optional)
    }

    /// An option is written as `some(value)` or `none`. The payload may also be
    /// written directly, unless it is itself an option.
    fn option<'n>(&self, node: &'n Node, payload: &Type) -> Result<Option<&'n Node>> {
        match node.ty() {
            NodeType::OptionSome | NodeType::OptionNone => self.parse(node.as_option()),
            _ if !self.is_option_or_result(payload) => Ok(Some(node)),
            _ => Err(self.error(
                node,
                "options of options or results must be written as `some(...)` or `none`",
            )),
        }
    }

    /// A result is written as `ok(value)` or `err(value)`, or `ok` or `err`
    /// for cases without a payload. An `ok` payload may also be written
    /// directly, unless it is an option or result.
    fn result_case<'n>(
        &self,
        node: &'n Node,
        r: &wit_parser::Result_,
    ) -> Result<(usize, Option<&'n Node>)> {
        let (index, label, payload) = match node.ty() {
            NodeType::ResultOk | NodeType::ResultErr => match self.parse(node.as_result())? {
                Ok(payload) => (0, "ok", payload),
                Err(payload) => (1, "err", payload),
            },
            _ => match &r.ok {
                Some(ok) if !self.is_option_or_result(ok) => return Ok((0, Some(node))),
                _ => {
                    return Err(
                        self.error(node, "results must be written as `ok(...)` or `err(...)`")
                    );
                }
            },
        };
        match ([&r.ok, &r.err][index], payload) {
            (Some(_), None) => Err(self.error(node, &format!("case `{label}` requires a payload"))),
            (None, Some(_)) => Err(self.error(node, &format!("case `{label}` has no payload"))),
            _ => Ok((index, payload)),
        }
    }

    fn is_option_or_result(&self, ty: &Type) -> bool {
        match dealias(self.resolve, *ty) {
            Type::Id(id) => matches!(
                self.resolve.types[id].kind,
                TypeDefKind::Option(_) | TypeDefKind::Result(_)
            ),
            _ => false,
        }
    }

    fn is_option(&self, ty: &Type) -> bool {
        match dealias(self.resolve, *ty) {
            Type::Id(id) => matches!(self.resolve.types[id].kind, TypeDefKind::Option(_)),
            _ => false,
        }
    }

    fn flags(&self, node: &Node, flags: &wit_parser::Flags) -> Result<u32> {
        let mut bits = 0u32;
        for label in self.parse(node.as_flags(self.src))? {
            let label = label.strip_prefix('%').unwrap_or(label);
            let Some(i) = flags.flags.iter().position(|f| f.name == label) else {
                return Err(self.error(node, &format!("unknown flag `{label}`")));
            };
            if bits & (1 << i) != 0 {
                return Err(self.error(node, &format!("duplicate flag `{label}`")));
            }
            bits |= 1 << i;
        }
        Ok(bits)
    }

    fn number<T: std::str::FromStr>(&self, node: &Node) -> Result<T> {
        self.parse(node.as_number(self.src)).map_err(|e| {
            // report the type using its WIT name, e.g. s32 rather than i32
            let name = std::any::type_name::<T>();
            let name = name.strip_prefix('i').map(|n| format!("s{n}"));
            e.context(format!(
                "expected {}",
                name.as_deref().unwrap_or(std::any::type_name::<T>())
            ))
        })
    }

    fn char(&self, node: &Node) -> Result<char> {
        self.parse(node.as_char(self.src))
    }

    fn parse<T>(&self, result: Result<T, ParserError>) -> Result<T> {
        result.map_err(|e| parser_error(self.src, e))
    }

    fn error(&self, node: &Node, msg: &str) -> Error {
        anyhow!("{msg} at {}", position(self.src, node.span()))
    }
}

/// Matches the fields of a WAVE record node to the expected names, returning
/// the value nodes in the order of `names`. Every field may be omitted.
pub fn optional_fields<'n>(
    src: &str,
    node: &'n Node,
    names: &[&str],
) -> Result<Vec<Option<&'n Node>>> {
    match_fields(src, node, names, &vec![true; names.len()])
}

/// Matches the fields of a WAVE record node to the expected names, returning
/// the value nodes in the order of `names`. Fields marked `optional` may be
/// omitted.
fn match_fields<'n>(
    src: &str,
    node: &'n Node,
    names: &[&str],
    optional: &[bool],
) -> Result<Vec<Option<&'n Node>>> {
    let mut values: Vec<Option<&Node>> = vec![None; names.len()];
    // `{}` parses as empty flags, but is also an empty record
    let empty =
        node.ty() == NodeType::Flags && node.as_flags(src).is_ok_and(|mut f| f.next().is_none());
    let fields: Vec<(&str, &Node)> = match empty {
        true => vec![],
        false => node
            .as_record(src)
            .map_err(|e| parser_error(src, e))?
            .collect(),
    };
    for (name, value) in fields {
        let Some(i) = names.iter().position(|n| *n == name) else {
            bail!(
                "unexpected field `{name}` at {}",
                position(src, value.span())
            );
        };
        if values[i].replace(value).is_some() {
            bail!(
                "duplicate field `{name}` at {}",
                position(src, value.span())
            );
        }
    }
    for ((name, value), optional) in names.iter().zip(&values).zip(optional) {
        if value.is_none() && !optional {
            bail!("missing field `{name}` at {}", position(src, node.span()));
        }
    }
    Ok(values)
}

pub fn parser_error(src: &str, e: ParserError) -> Error {
    let kind = e.kind();
    match e.detail() {
        Some(detail) => anyhow!("{kind}: {detail} at {}", position(src, e.span())),
        None => anyhow!("{kind} at {}", position(src, e.span())),
    }
}

/// Formats the start of a span as `line:column` (1-based).
fn position(src: &str, span: Range<usize>) -> String {
    let before = &src[..span.start.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("{line}:{column}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_wave::untyped::UntypedValue;

    fn flatten(wit: &str, src: &str) -> Vec<Flat> {
        let mut resolve = Resolve::default();
        let pkg = resolve.push_str("test.wit", wit).unwrap();
        let iface = resolve.packages[pkg].interfaces["i"];
        let ty = Type::Id(resolve.interfaces[iface].types["v"]);
        let value = UntypedValue::parse(src).unwrap();
        let mut layout = Layout::new(&resolve, 8).unwrap();
        layout.set_source(src);
        let mut out = vec![];
        layout.flatten(&ty, value.node(), &mut out).unwrap();
        out
    }

    #[test]
    fn it_joins_variant_payloads() {
        let wit = "package a:b; interface i { variant v { a(f32), b(u64), c(string), d } }";
        let as_debug = |src| format!("{:?}", flatten(wit, src));
        // slot types join to [i32 tag, i64, i32]
        assert_eq!(
            as_debug("a(1.5)"),
            format!(
                "{:?}",
                [
                    Flat::I32(0),
                    Flat::I64(1.5f32.to_bits() as i64),
                    Flat::I32(0)
                ]
            )
        );
        assert_eq!(
            as_debug("b(18446744073709551615)"),
            format!("{:?}", [Flat::I32(1), Flat::I64(-1), Flat::I32(0)])
        );
        assert_eq!(
            as_debug(r#"c("xy")"#),
            format!("{:?}", [Flat::I32(2), Flat::I64(8), Flat::I32(2)])
        );
        assert_eq!(
            as_debug("d"),
            format!("{:?}", [Flat::I32(3), Flat::I64(0), Flat::I32(0)])
        );
    }

    #[test]
    fn it_flattens_options() {
        let wit = "package a:b;
            interface i {
                record r { a: option<u8>, b: option<f64> }
            }";
        let mut resolve = Resolve::default();
        let pkg = resolve.push_str("test.wit", wit).unwrap();
        let iface = resolve.packages[pkg].interfaces["i"];
        let ty = Type::Id(resolve.interfaces[iface].types["r"]);
        let src = "{b: 1.5}";
        let value = UntypedValue::parse(src).unwrap();
        let mut layout = Layout::new(&resolve, 8).unwrap();
        layout.set_source(src);
        let mut out = vec![];
        layout.flatten(&ty, value.node(), &mut out).unwrap();
        assert_eq!(
            format!("{out:?}"),
            format!(
                "{:?}",
                [
                    Flat::I32(0),
                    Flat::I32(0),
                    Flat::I32(1),
                    Flat::F64(1.5f64.to_bits())
                ]
            )
        );
    }
}
