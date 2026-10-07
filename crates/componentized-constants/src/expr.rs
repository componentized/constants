//! Expressions generating the items of a stream, written as a subset of Rust's
//! closure syntax, e.g. `|a, b| a + b`. See
//! grammars/constants-expression/README.md for the full specification.
//!
//! The closure takes the previous items of the stream as parameters and returns
//! the next item. Besides its parameters, an expression may read `$init`, a
//! tuple of the function's arguments, and `$i`, the position in the stream of
//! the item being generated.
//!
//! Supported, following Rust's precedence and semantics:
//!
//! - integer, float and `true`/`false` literals, typed by their context
//! - the closure's parameters, `$init.0`, `$init.1`, …, and `$i`
//! - `-` and `!`, `!` is a bitwise not for integers
//! - `*`, `/`, `%`, `+`, `-`, `<<`, `>>`, `&`, `^`, `|`
//! - `==`, `!=`, `<`, `<=`, `>`, `>=`, `&&` and `||`
//! - `as` casts between numeric types, and from `bool` to integers
//! - `wrapping_add`, `wrapping_sub`, `wrapping_mul`, `rotate_left` and
//!   `rotate_right` methods on integers
//! - parentheses, `if c { a } else { b }`, including `else if`, and blocks
//!   with `let` bindings, e.g. `{ let z = a ^ b; z ^ z >> 3 }`
//!
//! Where Rust's checked arithmetic fails, e.g. overflow, division by zero or a
//! shift by at least the type's bits, or a float operation's result isn't
//! finite, the item can't be generated and the stream ends.

use anyhow::{Result, anyhow, bail};
use wasm_encoder::{BlockType, Function, InstructionSink, MemArg, ValType};
use wit_parser::{Resolve, Type};

use crate::streams::{INDEX, SEEDS};
use crate::types::dealias;
use crate::values::position;

/// The types expressions support.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    Bool,
    U8,
    S8,
    U16,
    S16,
    U32,
    S32,
    U64,
    S64,
    F32,
    F64,
}

impl Prim {
    pub fn from_type(resolve: &Resolve, ty: &Type) -> Option<Prim> {
        Some(match dealias(resolve, *ty) {
            Type::Bool => Prim::Bool,
            Type::U8 => Prim::U8,
            Type::S8 => Prim::S8,
            Type::U16 => Prim::U16,
            Type::S16 => Prim::S16,
            Type::U32 => Prim::U32,
            Type::S32 => Prim::S32,
            Type::U64 => Prim::U64,
            Type::S64 => Prim::S64,
            Type::F32 => Prim::F32,
            Type::F64 => Prim::F64,
            _ => return None,
        })
    }

    /// A type named in an expression, by its Rust name, or its WIT name for
    /// signed integers.
    fn from_name(name: &str) -> Option<Prim> {
        Some(match name {
            "bool" => Prim::Bool,
            "u8" => Prim::U8,
            "i8" | "s8" => Prim::S8,
            "u16" => Prim::U16,
            "i16" | "s16" => Prim::S16,
            "u32" => Prim::U32,
            "i32" | "s32" => Prim::S32,
            "u64" => Prim::U64,
            "i64" | "s64" => Prim::S64,
            "f32" => Prim::F32,
            "f64" => Prim::F64,
            _ => return None,
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Prim::Bool => "bool",
            Prim::U8 => "u8",
            Prim::S8 => "s8",
            Prim::U16 => "u16",
            Prim::S16 => "s16",
            Prim::U32 => "u32",
            Prim::S32 => "s32",
            Prim::U64 => "u64",
            Prim::S64 => "s64",
            Prim::F32 => "f32",
            Prim::F64 => "f64",
        }
    }

    fn is_float(self) -> bool {
        matches!(self, Prim::F32 | Prim::F64)
    }

    fn is_int(self) -> bool {
        !self.is_float() && self != Prim::Bool
    }

    fn signed(self) -> bool {
        matches!(self, Prim::S8 | Prim::S16 | Prim::S32 | Prim::S64)
    }

    /// Integers of at most 32 bits are i32s, sign or zero extended.
    fn narrow(self) -> bool {
        self.is_int() && self.bits() <= 32
    }

    fn wide(self) -> bool {
        self.val_type() == ValType::I64
    }

    fn bits(self) -> u32 {
        match self {
            Prim::Bool | Prim::U8 | Prim::S8 => 8,
            Prim::U16 | Prim::S16 => 16,
            Prim::U32 | Prim::S32 | Prim::F32 => 32,
            Prim::U64 | Prim::S64 | Prim::F64 => 64,
        }
    }

    fn range(self) -> (i128, i128) {
        let bits = self.bits();
        match self.signed() {
            true => (-(1 << (bits - 1)), (1 << (bits - 1)) - 1),
            false => (0, (1 << bits) - 1),
        }
    }

    pub(crate) fn val_type(self) -> ValType {
        match self {
            Prim::U64 | Prim::S64 => ValType::I64,
            Prim::F32 => ValType::F32,
            Prim::F64 => ValType::F64,
            _ => ValType::I32,
        }
    }

    /// The size of an item of this type in memory.
    pub fn size(self) -> u32 {
        match self {
            Prim::Bool => 1,
            _ => self.bits() / 8,
        }
    }

    fn mem_arg(self, offset: u32) -> MemArg {
        MemArg {
            offset: offset.into(),
            align: self.size().trailing_zeros(),
            memory_index: 0,
        }
    }

    fn load(self, i: &mut InstructionSink, offset: u32) {
        let m = self.mem_arg(offset);
        match self {
            Prim::Bool | Prim::U8 => i.i32_load8_u(m),
            Prim::S8 => i.i32_load8_s(m),
            Prim::U16 => i.i32_load16_u(m),
            Prim::S16 => i.i32_load16_s(m),
            Prim::U32 | Prim::S32 => i.i32_load(m),
            Prim::U64 | Prim::S64 => i.i64_load(m),
            Prim::F32 => i.f32_load(m),
            Prim::F64 => i.f64_load(m),
        };
    }

    pub(crate) fn store(self, i: &mut InstructionSink, offset: u32) {
        let m = self.mem_arg(offset);
        match self {
            Prim::Bool | Prim::U8 | Prim::S8 => i.i32_store8(m),
            Prim::U16 | Prim::S16 => i.i32_store16(m),
            Prim::U32 | Prim::S32 => i.i32_store(m),
            Prim::U64 | Prim::S64 => i.i64_store(m),
            Prim::F32 => i.f32_store(m),
            Prim::F64 => i.f64_store(m),
        };
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    WrapMul,
    WrapDiv,
    WrapRem,
    WrapAdd,
    WrapSub,
    WrapShl,
    WrapShr,
    /// `**`, exponentiation.
    Pow,
    /// `**%`, wrapping exponentiation.
    WrapPow,
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    BitAnd,
    BitXor,
    BitOr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::WrapMul => "*%",
            Op::WrapDiv => "/%",
            Op::WrapRem => "%%",
            Op::WrapAdd => "+%",
            Op::WrapSub => "-%",
            Op::WrapShl => "<<%",
            Op::WrapShr => ">>%",
            Op::Pow => "**",
            Op::WrapPow => "**%",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Rem => "%",
            Op::Add => "+",
            Op::Sub => "-",
            Op::Shl => "<<",
            Op::Shr => ">>",
            Op::BitAnd => "&",
            Op::BitXor => "^",
            Op::BitOr => "|",
            Op::Eq => "==",
            Op::Ne => "!=",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::And => "&&",
            Op::Or => "||",
        }
    }

    /// The wrapping arithmetic operators, e.g. `+%`, but not the shifts.
    fn is_wrapping(self) -> bool {
        matches!(
            self,
            Op::WrapMul | Op::WrapDiv | Op::WrapRem | Op::WrapAdd | Op::WrapSub
        )
    }

    fn is_shift(self) -> bool {
        matches!(self, Op::Shl | Op::Shr | Op::WrapShl | Op::WrapShr)
    }

    fn is_comparison(self) -> bool {
        matches!(self, Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    RotateLeft,
    RotateRight,
}

impl Method {
    fn from_name(name: &str) -> Option<Method> {
        Some(match name {
            "wrapping_add" => Method::WrappingAdd,
            "wrapping_sub" => Method::WrappingSub,
            "wrapping_mul" => Method::WrappingMul,
            "rotate_left" => Method::RotateLeft,
            "rotate_right" => Method::RotateRight,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Method::WrappingAdd => "wrapping_add",
            Method::WrappingSub => "wrapping_sub",
            Method::WrappingMul => "wrapping_mul",
            Method::RotateLeft => "rotate_left",
            Method::RotateRight => "rotate_right",
        }
    }

    fn is_rotate(self) -> bool {
        matches!(self, Method::RotateLeft | Method::RotateRight)
    }
}

enum Expr {
    Int(i128),
    Float(f64),
    Bool(bool),
    Param(u32),
    Local(u32),
    Seed(u32),
    /// `$init.name`, a field by the function's parameter's name, and where it
    /// is, for errors, resolved to its index before types are checked.
    SeedName(String, String),
    Index,
    Neg(Box<Expr>),
    /// `-%e`, wrapping negation.
    WrapNeg(Box<Expr>),
    Not(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Block(Vec<Stmt>, Box<Expr>),
    Cast(Box<Expr>, Prim),
    Method(Method, Box<Expr>, Box<Expr>),
    /// `[a, b, c]`, only as the closure's result, each element an item.
    Array(Vec<Expr>),
    /// `x.to_le_bytes()`, or `to_be_bytes` when big endian, only as the
    /// closure's result, each byte an item.
    Bytes(Box<Expr>, bool),
}

/// A statement in a block, before the block's value, if any.
enum Stmt {
    Let(Let),
    /// `x = value;`, a compound assignment, e.g. `x += 1;`, is `x = x + 1;`.
    Assign {
        local: u32,
        value: Expr,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    If {
        cond: Expr,
        then: Vec<Stmt>,
        otherwise: Vec<Stmt>,
    },
}

/// A block's statements, its value if it has one, and where it ends, for
/// errors.
struct Items {
    stmts: Vec<Stmt>,
    tail: Option<Expr>,
    end: String,
}

/// An `if` before it's known to be a value or a statement.
struct IfParts {
    cond: Expr,
    then: Items,
    otherwise: Else,
    end: String,
}

enum Else {
    None,
    If(Box<IfParts>),
    Block(Items),
}

impl IfParts {
    /// Whether every branch has a value, so the `if` is a value.
    fn has_values(&self) -> bool {
        self.then.tail.is_some()
            && match &self.otherwise {
                Else::None => false,
                Else::If(parts) => parts.has_values(),
                Else::Block(items) => items.tail.is_some(),
            }
    }

    fn into_expr(self) -> Result<Expr> {
        let then = items_value(self.then)?;
        let otherwise = match self.otherwise {
            Else::None => bail!(
                "expected `else`, an `if` needs a value for both branches {}",
                self.end
            ),
            Else::If(parts) => parts.into_expr()?,
            Else::Block(items) => items_value(items)?,
        };
        Ok(Expr::If(
            Box::new(self.cond),
            Box::new(then),
            Box::new(otherwise),
        ))
    }

    fn into_stmt(self) -> Result<Stmt> {
        let then = items_stmts(self.then)?;
        let otherwise = match self.otherwise {
            Else::None => vec![],
            Else::If(parts) => vec![parts.into_stmt()?],
            Else::Block(items) => items_stmts(items)?,
        };
        Ok(Stmt::If {
            cond: self.cond,
            then,
            otherwise,
        })
    }
}

/// A block as a value, which must end with one.
fn items_value(items: Items) -> Result<Expr> {
    let Some(tail) = items.tail else {
        bail!("expected a value at the end of the block {}", items.end);
    };
    Ok(match items.stmts.is_empty() {
        true => tail,
        false => Expr::Block(items.stmts, Box::new(tail)),
    })
}

/// A block of statements, a loop's or an `if` statement's, without a value.
fn items_stmts(items: Items) -> Result<Vec<Stmt>> {
    if items.tail.is_some() {
        bail!(
            "a `while` loop's or an `if` statement's block can't end with a value {}",
            items.end
        );
    }
    Ok(items.stmts)
}

/// `let name: ty = init;`, binding a local, or nothing for `_`.
struct Let {
    local: Option<u32>,
    ty: Option<Prim>,
    init: Expr,
}

/// A parsed closure, before its types are checked.
pub struct Closure {
    params: u32,
    locals: u32,
    body: Expr,
    /// Whether the expression reads `$init`, the function's arguments.
    pub uses_seed: bool,
}

/// An expression generating the next item of a stream from the previous
/// `params` items, the function's arguments in `seeds`, and the position.
pub struct Generator {
    pub params: u32,
    /// The number of items each evaluation generates, more than one when the
    /// closure returns an array.
    pub count: u32,
    pub item: Prim,
    pub seeds: Vec<Prim>,
    locals: Vec<Prim>,
    body: Expr,
}

/// Parses a closure, without checking its types.
///
/// The function's arguments, named in `args` as Rust names them, e.g.
/// `start_value` for `start-value`, are in scope outside the closure's
/// parameters, which shadow them, as do `let` bindings. Reading one reads
/// `$init`.
pub fn parse_closure<'a>(src: &'a str, args: &'a [String]) -> Result<Closure> {
    let mut parser = Parser {
        src,
        tokens: tokenize(src)?,
        pos: 0,
        scope: args
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), Binding::Arg(index as u32)))
            .collect(),
        params: 0,
        param_names: vec![],
        locals: 0,
        uses_seed: false,
    };
    let body = parser.closure()?;
    if let Some(token) = parser.tokens.get(parser.pos) {
        bail!("unexpected `{}` at {}", token.text(src), token.at(src));
    }
    Ok(Closure {
        params: parser.params,
        locals: parser.locals,
        body,
        uses_seed: parser.uses_seed,
    })
}

impl Closure {
    /// Checks the closure generates items of type `item`, where `$init` holds
    /// arguments of the types in `seeds`, of the parameters named `names`, as
    /// Rust names them, e.g. `start_value` for `start-value`.
    pub fn check(mut self, item: Prim, seeds: &[Prim], names: &[String]) -> Result<Generator> {
        resolve_seed_names(&mut self.body, names)?;
        let mut generator = Generator {
            params: self.params,
            count: 1,
            item,
            seeds: seeds.to_vec(),
            locals: vec![Prim::Bool; self.locals as usize],
            body: Expr::Bool(false),
        };
        generator.type_locals(&self.body)?;
        let body = self.body;
        generator.count = generator.check_result(&body)?;
        generator.body = body;
        Ok(generator)
    }
}

/// Replaces each `$init.name` with the index of the parameter it names.
fn resolve_seed_names(e: &mut Expr, names: &[String]) -> Result<()> {
    match e {
        Expr::SeedName(name, at) => {
            let Some(index) = names.iter().position(|n| n == name) else {
                let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                bail!(
                    "`$init` has no field `{name}` at {at}, the function's parameters are {}",
                    match names.is_empty() {
                        true => "none".to_string(),
                        false => names.join(", "),
                    }
                );
            };
            *e = Expr::Seed(index as u32);
        }
        Expr::Neg(e) | Expr::WrapNeg(e) | Expr::Not(e) | Expr::Cast(e, _) | Expr::Bytes(e, _) => {
            resolve_seed_names(e, names)?
        }
        Expr::Binary(_, l, r) | Expr::Method(_, l, r) => {
            resolve_seed_names(l, names)?;
            resolve_seed_names(r, names)?;
        }
        Expr::If(c, t, e) => {
            for e in [c, t, e] {
                resolve_seed_names(e, names)?;
            }
        }
        Expr::Block(stmts, e) => {
            resolve_stmt_seed_names(stmts, names)?;
            resolve_seed_names(e, names)?;
        }
        Expr::Array(elements) => {
            for e in elements {
                resolve_seed_names(e, names)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_stmt_seed_names(stmts: &mut [Stmt], names: &[String]) -> Result<()> {
    for stmt in stmts {
        match stmt {
            Stmt::Let(l) => resolve_seed_names(&mut l.init, names)?,
            Stmt::Assign { value, .. } => resolve_seed_names(value, names)?,
            Stmt::While { cond, body } => {
                resolve_seed_names(cond, names)?;
                resolve_stmt_seed_names(body, names)?;
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                resolve_seed_names(cond, names)?;
                resolve_stmt_seed_names(then, names)?;
                resolve_stmt_seed_names(otherwise, names)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ident,
    Var,
    Int(u128),
    Float(f64),
    Punct(&'static str),
}

#[derive(Clone, Copy)]
struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Token {
    fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }

    fn at(&self, src: &str) -> String {
        position(src, self.start..self.end)
    }
}

/// Punctuation, longest first so `||` isn't read as two `|`.
const PUNCT: [&str; 59] = [
    "**%=", "<<%=", ">>%=", "**%", "**=", "<<%", ">>%", "<<=", ">>=", "+%=", "-%=", "*%=", "/%=",
    "%%=", "**", "&&", "||", "<<", ">>", "<=", ">=", "==", "!=", "+%", "-%", "*%", "/%", "%%",
    "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "+", "-", "*", "/", "%", "!", "&", "|", "^",
    "<", ">", "=", "(", ")", "{", "}", "[", "]", ",", ";", ":", ".", "_",
];

/// Assignment operators, `=` and compound assignments, e.g. `+=`.
const ASSIGN: [(&str, Option<Op>); 20] = [
    ("=", None),
    ("+=", Some(Op::Add)),
    ("-=", Some(Op::Sub)),
    ("*=", Some(Op::Mul)),
    ("/=", Some(Op::Div)),
    ("%=", Some(Op::Rem)),
    ("<<=", Some(Op::Shl)),
    (">>=", Some(Op::Shr)),
    ("&=", Some(Op::BitAnd)),
    ("^=", Some(Op::BitXor)),
    ("|=", Some(Op::BitOr)),
    ("+%=", Some(Op::WrapAdd)),
    ("-%=", Some(Op::WrapSub)),
    ("*%=", Some(Op::WrapMul)),
    ("/%=", Some(Op::WrapDiv)),
    ("%%=", Some(Op::WrapRem)),
    ("<<%=", Some(Op::WrapShl)),
    (">>%=", Some(Op::WrapShr)),
    ("**=", Some(Op::Pow)),
    ("**%=", Some(Op::WrapPow)),
];

fn tokenize(src: &str) -> Result<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut tokens = vec![];
    let mut pos = 0;
    while pos < bytes.len() {
        let c = bytes[pos];
        let start = pos;
        if c.is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        let kind = if c.is_ascii_alphabetic() || (c == b'_' && is_ident(bytes.get(pos + 1))) {
            while is_ident(bytes.get(pos)) {
                pos += 1;
            }
            Kind::Ident
        } else if c == b'$' && is_ident(bytes.get(pos + 1)) {
            pos += 1;
            while is_ident(bytes.get(pos)) {
                pos += 1;
            }
            Kind::Var
        } else if c.is_ascii_digit() {
            number(src, &mut pos)?
        } else if let Some(p) = PUNCT.into_iter().find(|p| src[pos..].starts_with(p)) {
            pos += p.len();
            Kind::Punct(p)
        } else {
            let c = src[pos..].chars().next().expect("a char");
            bail!(
                "unexpected `{c}` at {}",
                position(src, pos..pos + c.len_utf8())
            );
        };
        tokens.push(Token {
            kind,
            start,
            end: pos,
        });
    }
    Ok(tokens)
}

fn is_ident(c: Option<&u8>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// An integer, in decimal, hex `0x`, octal `0o` or binary `0b`, or a decimal
/// float, digits may be separated by `_`.
fn number(src: &str, pos: &mut usize) -> Result<Kind> {
    let bytes = src.as_bytes();
    let start = *pos;
    let digits = |pos: &mut usize, radix: u32| {
        while bytes
            .get(*pos)
            .is_some_and(|c| *c == b'_' || (*c as char).is_digit(radix))
        {
            *pos += 1;
        }
    };
    let radix = match (bytes[*pos], bytes.get(*pos + 1)) {
        (b'0', Some(b'x')) => 16,
        (b'0', Some(b'o')) => 8,
        (b'0', Some(b'b')) => 2,
        _ => 10,
    };
    let at = || position(src, start..start + 1);
    if radix != 10 {
        *pos += 2;
        digits(pos, radix);
        if is_ident(bytes.get(*pos)) {
            bail!("literal suffixes aren't supported at {}", at());
        }
        let text: String = src[start + 2..*pos].chars().filter(|c| *c != '_').collect();
        return u128::from_str_radix(&text, radix)
            .map(Kind::Int)
            .map_err(|_| anyhow!("invalid integer at {}", at()));
    }
    digits(pos, 10);
    let mut float = false;
    // a `.` followed by a digit, or by nothing that could start a field or
    // method, makes a float, e.g. `1.5` or `1.`
    if bytes.get(*pos) == Some(&b'.')
        && !bytes
            .get(*pos + 1)
            .is_some_and(|c| *c == b'.' || *c == b'_' || c.is_ascii_alphabetic())
    {
        float = true;
        *pos += 1;
        digits(pos, 10);
    }
    if matches!(bytes.get(*pos), Some(b'e' | b'E')) {
        float = true;
        *pos += 1;
        if matches!(bytes.get(*pos), Some(b'+' | b'-')) {
            *pos += 1;
        }
        digits(pos, 10);
    }
    if is_ident(bytes.get(*pos)) {
        bail!("literal suffixes aren't supported at {}", at());
    }
    let text: String = src[start..*pos].chars().filter(|c| *c != '_').collect();
    match float {
        true => text
            .parse()
            .map(Kind::Float)
            .map_err(|_| anyhow!("invalid float at {}", at())),
        false => text
            .parse()
            .map(Kind::Int)
            .map_err(|_| anyhow!("invalid integer at {}", at())),
    }
}

/// What a name in scope refers to.
#[derive(Clone, Copy)]
enum Binding {
    /// One of the function's arguments, which are never assigned.
    Arg(u32),
    Param(u32),
    /// A `let` binding, and whether it's `let mut`.
    Local(u32, bool),
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    /// Names in scope, later names shadowing earlier ones.
    scope: Vec<(&'a str, Binding)>,
    params: u32,
    /// The closure's parameters' names, `None` for `_`, for `$call`'s fields.
    param_names: Vec<Option<&'a str>>,
    locals: u32,
    uses_seed: bool,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<Kind> {
        self.tokens.get(self.pos).map(|t| t.kind)
    }

    fn peek_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Kind::Punct(q)) if q == p)
    }

    fn peek_ident(&self, name: &str) -> bool {
        self.tokens
            .get(self.pos)
            .is_some_and(|t| t.kind == Kind::Ident && t.text(self.src) == name)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        let found = self.peek_punct(p);
        if found {
            self.pos += 1;
        }
        found
    }

    fn expect_punct(&mut self, p: &str) -> Result<()> {
        if !self.eat_punct(p) {
            bail!("expected `{p}` {}", self.at());
        }
        Ok(())
    }

    /// Describes where the next token is, for errors.
    fn at(&self) -> String {
        match self.tokens.get(self.pos) {
            Some(token) => format!(
                "at {}, found `{}`",
                token.at(self.src),
                token.text(self.src)
            ),
            None => "at the end".to_string(),
        }
    }

    /// A name to bind, a parameter or a `let`, or `None` for `_`.
    fn binding_name(&mut self, what: &str) -> Result<Option<&'a str>> {
        let token = self.tokens.get(self.pos).copied();
        match token {
            Some(t) if t.kind == Kind::Var => bail!(
                "`{}` can't be bound at {}, `$init`, `$call` and `$i` are built in, and can't \
                 be shadowed",
                t.text(self.src),
                t.at(self.src)
            ),
            Some(t) if t.kind == Kind::Punct("_") => {
                self.pos += 1;
                Ok(None)
            }
            Some(t) if t.kind == Kind::Ident && !is_keyword(t.text(self.src)) => {
                self.pos += 1;
                Ok(Some(t.text(self.src)))
            }
            _ => bail!("expected {what} {}", self.at()),
        }
    }

    /// `|a, b| expr`, or `|| expr` without parameters.
    fn closure(&mut self) -> Result<Expr> {
        if !self.eat_punct("||") {
            self.expect_punct("|")?;
            while !self.eat_punct("|") {
                let token = self.tokens[self.pos.min(self.tokens.len().saturating_sub(1))];
                let name = self.binding_name("a parameter name")?;
                if let Some(name) = name {
                    // only another parameter conflicts, an argument is shadowed
                    if self
                        .scope
                        .iter()
                        .any(|(n, b)| *n == name && matches!(b, Binding::Param(_)))
                    {
                        bail!("duplicate parameter `{name}` at {}", token.at(self.src));
                    }
                    self.scope.push((name, Binding::Param(self.params)));
                }
                self.param_names.push(name);
                self.params += 1;
                if !self.eat_punct(",") && !self.peek_punct("|") {
                    bail!("expected `,` or `|` {}", self.at());
                }
            }
        }
        self.expr()
    }

    fn expr(&mut self) -> Result<Expr> {
        self.binary(0)
    }

    /// Binary operators by precedence, lowest first. Comparisons don't chain.
    fn binary(&mut self, level: usize) -> Result<Expr> {
        const LEVELS: [&[(&str, Op)]; 9] = [
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[
                ("==", Op::Eq),
                ("!=", Op::Ne),
                ("<=", Op::Le),
                (">=", Op::Ge),
                ("<", Op::Lt),
                (">", Op::Gt),
            ],
            &[("|", Op::BitOr)],
            &[("^", Op::BitXor)],
            &[("&", Op::BitAnd)],
            &[
                ("<<", Op::Shl),
                (">>", Op::Shr),
                ("<<%", Op::WrapShl),
                (">>%", Op::WrapShr),
            ],
            &[
                ("+", Op::Add),
                ("-", Op::Sub),
                ("+%", Op::WrapAdd),
                ("-%", Op::WrapSub),
            ],
            &[
                ("*", Op::Mul),
                ("/", Op::Div),
                ("%", Op::Rem),
                ("*%", Op::WrapMul),
                ("/%", Op::WrapDiv),
                ("%%", Op::WrapRem),
            ],
        ];
        let Some(ops) = LEVELS.get(level) else {
            return self.cast();
        };
        let mut lhs = self.binary(level + 1)?;
        while let Some((_, op)) = ops.iter().find(|(p, _)| self.peek_punct(p)) {
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(*op, Box::new(lhs), Box::new(rhs));
            if op.is_comparison() && ops.iter().any(|(p, _)| self.peek_punct(p)) {
                bail!("comparison operators can't be chained {}", self.at());
            }
        }
        Ok(lhs)
    }

    /// `e as ty`, binding tighter than binary operators and looser than unary.
    fn cast(&mut self) -> Result<Expr> {
        let mut e = self.unary()?;
        while self.peek_ident("as") {
            self.pos += 1;
            let ty = self.type_name()?;
            e = Expr::Cast(Box::new(e), ty);
        }
        Ok(e)
    }

    fn type_name(&mut self) -> Result<Prim> {
        let token = self.tokens.get(self.pos).copied();
        match token.and_then(|t| (t.kind == Kind::Ident).then(|| Prim::from_name(t.text(self.src))))
        {
            Some(Some(ty)) => {
                self.pos += 1;
                Ok(ty)
            }
            _ => bail!("expected a type, e.g. `u8`, `i32` or `f64`, {}", self.at()),
        }
    }

    fn unary(&mut self) -> Result<Expr> {
        if self.eat_punct("-") {
            return Ok(match self.unary()? {
                // a negative literal, so the minimum of a signed type is in range
                Expr::Int(v) => Expr::Int(-v),
                Expr::Float(v) => Expr::Float(-v),
                e => Expr::Neg(Box::new(e)),
            });
        }
        if self.eat_punct("-%") {
            return Ok(Expr::WrapNeg(Box::new(self.unary()?)));
        }
        if self.eat_punct("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.power()
    }

    /// `base ** exponent` or `base **% exponent`, binding tighter than a unary
    /// operator on its left, so `-x ** 2` is `-(x ** 2)`. The exponent is a
    /// unary expression, which makes `**` right associative, `a ** b ** c` is
    /// `a ** (b ** c)`, and allows `x ** -1`.
    fn power(&mut self) -> Result<Expr> {
        let base = self.postfix()?;
        for (punct, op) in [("**", Op::Pow), ("**%", Op::WrapPow)] {
            if self.eat_punct(punct) {
                let exponent = self.unary()?;
                return Ok(Expr::Binary(op, Box::new(base), Box::new(exponent)));
            }
        }
        Ok(base)
    }

    /// Method calls, `e.method(arg)`.
    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        while self.peek_punct(".") {
            let dot = self.tokens[self.pos];
            self.pos += 1;
            let token = self.tokens.get(self.pos).copied();
            let Some(name) = token
                .filter(|t| t.kind == Kind::Ident)
                .map(|t| t.text(self.src))
            else {
                bail!("expected a method name {}", self.at());
            };
            let bytes = match name {
                "to_le_bytes" => Some(false),
                "to_be_bytes" => Some(true),
                _ => None,
            };
            let method = Method::from_name(name);
            if bytes.is_none() && method.is_none() {
                bail!(
                    "unknown method `{name}` at {}, the methods are `wrapping_add`, \
                     `wrapping_sub`, `wrapping_mul`, `rotate_left`, `rotate_right`, \
                     `to_le_bytes` and `to_be_bytes`",
                    token.expect("a name").at(self.src)
                );
            }
            if matches!(e, Expr::Int(_) | Expr::Float(_)) {
                bail!(
                    "can't call `{name}` on a literal at {}, its type is ambiguous",
                    dot.at(self.src)
                );
            }
            self.pos += 1;
            self.expect_punct("(")?;
            e = match (method, bytes) {
                (Some(method), _) => {
                    if self.peek_punct(")") {
                        bail!("`{name}` takes an argument {}", self.at());
                    }
                    let arg = self.expr()?;
                    Expr::Method(method, Box::new(e), Box::new(arg))
                }
                (None, Some(big)) => {
                    if !self.peek_punct(")") {
                        bail!("`{name}` takes no arguments {}", self.at());
                    }
                    Expr::Bytes(Box::new(e), big)
                }
                (None, None) => unreachable!("checked above"),
            };
            self.expect_punct(")")?;
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr> {
        let Some(token) = self.tokens.get(self.pos).copied() else {
            bail!("expected an expression at the end");
        };
        match token.kind {
            Kind::Int(v) => {
                self.pos += 1;
                match i128::try_from(v) {
                    Ok(v) => Ok(Expr::Int(v)),
                    Err(_) => bail!("literal {v} is too large at {}", token.at(self.src)),
                }
            }
            Kind::Float(v) => {
                self.pos += 1;
                Ok(Expr::Float(v))
            }
            Kind::Punct("(") => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Kind::Punct("{") => self.block(),
            Kind::Punct("[") => {
                self.pos += 1;
                let mut elements = vec![];
                while !self.eat_punct("]") {
                    elements.push(self.expr()?);
                    if !self.eat_punct(",") && !self.peek_punct("]") {
                        bail!("expected `,` or `]` {}", self.at());
                    }
                }
                if elements.is_empty() {
                    bail!(
                        "an array must have at least one item at {}",
                        token.at(self.src)
                    );
                }
                Ok(Expr::Array(elements))
            }
            Kind::Var => {
                self.pos += 1;
                match token.text(self.src) {
                    "$i" => Ok(Expr::Index),
                    "$init" => {
                        self.uses_seed = true;
                        if !self.eat_punct(".") {
                            bail!(
                                "expected `.` and a field, `$init` is a tuple of the \
                                 function's arguments, e.g. `$init.0`, {}",
                                self.at()
                            );
                        }
                        let digits = self
                            .tokens
                            .get(self.pos)
                            .is_some_and(|t| t.text(self.src).bytes().all(|c| c.is_ascii_digit()));
                        let field = self.tokens.get(self.pos).copied();
                        match self.peek() {
                            Some(Kind::Int(index)) if digits && index <= u32::MAX as u128 => {
                                self.pos += 1;
                                Ok(Expr::Seed(index as u32))
                            }
                            Some(Kind::Ident) => {
                                let field = field.expect("a field");
                                self.pos += 1;
                                Ok(Expr::SeedName(
                                    field.text(self.src).to_string(),
                                    field.at(self.src),
                                ))
                            }
                            _ => bail!(
                                "expected a field of `$init`, an index, e.g. `0`, or a \
                                 parameter's name, {}",
                                self.at()
                            ),
                        }
                    }
                    "$call" => self.call_field(),
                    name => bail!(
                        "unknown variable `{name}` at {}, the variables are `$init`, `$call` and \
                         `$i`",
                        token.at(self.src)
                    ),
                }
            }
            Kind::Ident => match token.text(self.src) {
                "true" => {
                    self.pos += 1;
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.pos += 1;
                    Ok(Expr::Bool(false))
                }
                "if" => {
                    self.pos += 1;
                    self.if_parts()?.into_expr()
                }
                name if is_keyword(name) => bail!("expected an expression {}", self.at()),
                name => match self.scope.iter().rev().find(|(n, _)| *n == name) {
                    Some((_, binding)) => {
                        self.pos += 1;
                        Ok(match *binding {
                            Binding::Arg(index) => {
                                self.uses_seed = true;
                                Expr::Seed(index)
                            }
                            Binding::Param(index) => Expr::Param(index),
                            Binding::Local(id, _) => Expr::Local(id),
                        })
                    }
                    None => bail!("unknown name `{name}` at {}", token.at(self.src)),
                },
            },
            _ => bail!("expected an expression {}", self.at()),
        }
    }

    /// `$call.0` or `$call.name`, after the `$call`, one of the closure's
    /// parameters, the arguments of the current item's evaluation, which can't
    /// be shadowed.
    fn call_field(&mut self) -> Result<Expr> {
        if !self.eat_punct(".") {
            bail!(
                "expected `.` and a field, `$call` is the closure's parameters, e.g. \
                 `$call.0`, {}",
                self.at()
            );
        }
        let Some(field) = self.tokens.get(self.pos).copied() else {
            bail!("expected a field of `$call` at the end");
        };
        let text = field.text(self.src);
        let index = match field.kind {
            Kind::Int(index) if text.bytes().all(|c| c.is_ascii_digit()) => {
                if index >= self.params as u128 {
                    bail!(
                        "`$call.{index}` doesn't exist at {}, `$call` has {} fields, one for each \
                         of the closure's parameters",
                        field.at(self.src),
                        self.params
                    );
                }
                index as u32
            }
            Kind::Ident => match self.param_names.iter().position(|n| *n == Some(text)) {
                Some(index) => index as u32,
                None => {
                    let names: Vec<String> = self
                        .param_names
                        .iter()
                        .flatten()
                        .map(|n| format!("`{n}`"))
                        .collect();
                    bail!(
                        "`$call` has no field `{text}` at {}, the closure's parameters are {}",
                        field.at(self.src),
                        match names.is_empty() {
                            true => "unnamed".to_string(),
                            false => names.join(", "),
                        }
                    );
                }
            },
            _ => bail!(
                "expected a field of `$call`, an index, e.g. `0`, or a parameter's name, {}",
                self.at()
            ),
        };
        self.pos += 1;
        Ok(Expr::Param(index))
    }

    /// `if c { … } else { … }`, after the `if`, a value or a statement.
    fn if_parts(&mut self) -> Result<IfParts> {
        let cond = self.expr()?;
        if !self.peek_punct("{") {
            bail!("expected `{{` {}", self.at());
        }
        let then = self.items()?;
        let end = self.at();
        let otherwise = match self.peek_ident("else") {
            false => Else::None,
            true => {
                self.pos += 1;
                if self.peek_ident("if") {
                    self.pos += 1;
                    Else::If(Box::new(self.if_parts()?))
                } else {
                    if !self.peek_punct("{") {
                        bail!("expected `{{` or `if` {}", self.at());
                    }
                    Else::Block(self.items()?)
                }
            }
        };
        Ok(IfParts {
            cond,
            then,
            otherwise,
            end,
        })
    }

    /// A block as a value, e.g. `{ let a = …; a + 1 }`.
    fn block(&mut self) -> Result<Expr> {
        let items = self.items()?;
        items_value(items)
    }

    /// `{ statements… value }`, the value is optional. The bindings are in
    /// scope until the end of the block.
    fn items(&mut self) -> Result<Items> {
        self.expect_punct("{")?;
        let scope = self.scope.len();
        let mut stmts = vec![];
        let tail = loop {
            if self.peek_punct("}") {
                break None;
            }
            if self.peek_ident("let") {
                stmts.push(self.let_stmt()?);
                continue;
            }
            if self.peek_ident("while") {
                self.pos += 1;
                let cond = self.expr()?;
                if !self.peek_punct("{") {
                    bail!("expected `{{` {}", self.at());
                }
                let body = items_stmts(self.items()?)?;
                self.eat_punct(";");
                stmts.push(Stmt::While { cond, body });
                continue;
            }
            if self.peek_ident("if") {
                self.pos += 1;
                let parts = self.if_parts()?;
                // an `if` with values is the block's value, otherwise a statement
                if parts.has_values() {
                    if !self.peek_punct("}") {
                        bail!(
                            "expected `}}`, an `if` with values must be the block's value {}",
                            self.at()
                        );
                    }
                    break Some(parts.into_expr()?);
                }
                stmts.push(parts.into_stmt()?);
                self.eat_punct(";");
                continue;
            }
            if let Some(stmt) = self.assignment()? {
                stmts.push(stmt);
                continue;
            }
            let e = self.expr()?;
            if !self.peek_punct("}") {
                bail!("expected `}}` {}", self.at());
            }
            break Some(e);
        };
        let end = self.at();
        self.expect_punct("}")?;
        self.scope.truncate(scope);
        Ok(Items { stmts, tail, end })
    }

    /// `let name: ty = init;`, or `let mut …`, the binding is in scope after its
    /// initializer.
    fn let_stmt(&mut self) -> Result<Stmt> {
        self.pos += 1;
        let mutable = self.peek_ident("mut");
        if mutable {
            self.pos += 1;
        }
        let name = self.binding_name("a name to bind")?;
        let ty = match self.eat_punct(":") {
            true => Some(self.type_name()?),
            false => None,
        };
        self.expect_punct("=")?;
        let init = self.expr()?;
        self.expect_punct(";")?;
        let local = name.map(|name| {
            let id = self.locals;
            self.locals += 1;
            self.scope.push((name, Binding::Local(id, mutable)));
            id
        });
        Ok(Stmt::Let(Let { local, ty, init }))
    }

    /// `x = value;` or a compound assignment, e.g. `x += 1;`, to a `let mut`
    /// binding, when the next tokens are a name and an assignment operator.
    fn assignment(&mut self) -> Result<Option<Stmt>> {
        let Some(token) = self.tokens.get(self.pos).copied() else {
            return Ok(None);
        };
        // `$i = …;`, `$init.n = …;` or `$call.n = …;`, the built in variables never
        // change
        if token.kind == Kind::Var {
            let field = matches!(
                self.tokens.get(self.pos + 1).map(|t| t.kind),
                Some(Kind::Punct("."))
            );
            let next = self.tokens.get(self.pos + if field { 3 } else { 1 });
            if let Some(Kind::Punct(punct)) = next.map(|t| t.kind)
                && ASSIGN.iter().any(|(p, _)| *p == punct)
            {
                bail!(
                    "`{}` can't be assigned at {}, `$init`, `$call` and `$i` are built in, and \
                     never change, bind a copy with `let mut`",
                    token.text(self.src),
                    token.at(self.src)
                );
            }
            return Ok(None);
        }
        let Some(Kind::Punct(punct)) = self.tokens.get(self.pos + 1).map(|t| t.kind) else {
            return Ok(None);
        };
        let Some((_, op)) = ASSIGN.iter().find(|(p, _)| *p == punct) else {
            return Ok(None);
        };
        if token.kind != Kind::Ident || is_keyword(token.text(self.src)) {
            return Ok(None);
        }
        let name = token.text(self.src);
        let at = token.at(self.src);
        let id = match self.scope.iter().rev().find(|(n, _)| *n == name) {
            Some((_, Binding::Local(id, true))) => *id,
            Some((_, Binding::Local(_, false))) => {
                bail!("can't assign to `{name}` at {at}, declare it with `let mut {name}`")
            }
            Some((_, Binding::Param(_))) => bail!(
                "can't assign to the parameter `{name}` at {at}, bind a copy with \
                 `let mut {name} = {name};`"
            ),
            Some((_, Binding::Arg(_))) => bail!(
                "can't assign to the argument `{name}` at {at}, arguments never change, bind \
                 a copy with `let mut {name} = {name};`"
            ),
            None => bail!("unknown name `{name}` at {at}"),
        };
        self.pos += 2;
        let value = self.expr()?;
        self.expect_punct(";")?;
        let value = match op {
            None => value,
            Some(op) => Expr::Binary(*op, Box::new(Expr::Local(id)), Box::new(value)),
        };
        Ok(Some(Stmt::Assign { local: id, value }))
    }
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else" | "true" | "false" | "let" | "as" | "while" | "mut"
    )
}

/// The type of an expression, known from the expression itself, or a literal
/// whose type comes from its context.
#[derive(Clone, Copy)]
enum Synth {
    Known(Prim),
    Int,
    Float,
}

impl Synth {
    /// The type of an expression whose context doesn't decide it, literals
    /// default to `i32` or `f64`, as in Rust.
    fn resolve(self) -> Prim {
        match self {
            Synth::Known(ty) => ty,
            Synth::Int => Prim::S32,
            Synth::Float => Prim::F64,
        }
    }
}

/// The parameters of the function generating an item, then temporaries for the
/// operands and result of an operation, then for exponentiation's loop, then
/// the `let` bindings.
const STATE: u32 = 0;
const PTR: u32 = 1;
const A32: u32 = 2;
const B32: u32 = 3;
const A64: u32 = 4;
const B64: u32 = 5;
const R64: u32 = 6;
const F32: u32 = 7;
const F64: u32 = 8;
const POW_NEGATIVE: u32 = 9;
const POW_RESULT_32: u32 = 10;
const POW_BASE_32: u32 = 11;
const POW_EXPONENT: u32 = 12;
const POW_RESULT_64: u32 = 13;
const POW_BASE_64: u32 = 14;
const POW_RESULT_F32: u32 = 15;
const POW_BASE_F32: u32 = 16;
const POW_RESULT_F64: u32 = 17;
const POW_BASE_F64: u32 = 18;
const FIRST_LOCAL: u32 = 19;

impl Generator {
    fn seed(&self, field: u32) -> Result<Prim> {
        match self.seeds.get(field as usize) {
            Some(ty) => Ok(*ty),
            None => bail!(
                "`$init.{field}` doesn't exist, `$init` has {} fields, one for each of the \
                 function's arguments",
                self.seeds.len()
            ),
        }
    }

    /// Assigns the `let` bindings of statements their types, in order.
    fn type_stmts(&mut self, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Let(l) => {
                    self.type_locals(&l.init)?;
                    let ty = match (l.ty, self.synth(&l.init)?) {
                        (Some(ty), _) => ty,
                        (None, Synth::Known(ty)) => ty,
                        // only literals, `let x = 1;`
                        (None, _) => match l.local {
                            Some(_) => bail!(
                                "the type of a `let` bound to a literal must be given, e.g. \
                                 `let x: u32 = 1;`"
                            ),
                            None => Prim::Bool,
                        },
                    };
                    if let Some(id) = l.local {
                        self.locals[id as usize] = ty;
                    }
                }
                Stmt::Assign { value, .. } => self.type_locals(value)?,
                Stmt::While { cond, body } => {
                    self.type_locals(cond)?;
                    self.type_stmts(body)?;
                }
                Stmt::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    self.type_locals(cond)?;
                    self.type_stmts(then)?;
                    self.type_stmts(otherwise)?;
                }
            }
        }
        Ok(())
    }

    /// Assigns each `let` binding its type, in the order they're bound.
    fn type_locals(&mut self, e: &Expr) -> Result<()> {
        match e {
            Expr::Block(stmts, e) => {
                self.type_stmts(stmts)?;
                self.type_locals(e)
            }
            Expr::Neg(e)
            | Expr::WrapNeg(e)
            | Expr::Not(e)
            | Expr::Cast(e, _)
            | Expr::Bytes(e, _) => self.type_locals(e),
            Expr::Array(elements) => elements.iter().try_for_each(|e| self.type_locals(e)),
            Expr::Binary(_, l, r) | Expr::Method(_, l, r) => {
                self.type_locals(l)?;
                self.type_locals(r)
            }
            Expr::If(c, t, e) => {
                self.type_locals(c)?;
                self.type_locals(t)?;
                self.type_locals(e)
            }
            _ => Ok(()),
        }
    }

    /// The type of `e` from `e` alone, before considering its context.
    fn synth(&self, e: &Expr) -> Result<Synth> {
        Ok(match e {
            Expr::Int(_) => Synth::Int,
            Expr::Float(_) => Synth::Float,
            Expr::Bool(_) => Synth::Known(Prim::Bool),
            Expr::Param(_) => Synth::Known(self.item),
            Expr::Local(id) => Synth::Known(self.locals[*id as usize]),
            Expr::SeedName(..) => unreachable!("resolved before types are checked"),
            Expr::Seed(field) => Synth::Known(self.seed(*field)?),
            Expr::Index => Synth::Known(Prim::U64),
            Expr::Neg(e) | Expr::WrapNeg(e) | Expr::Not(e) => self.synth(e)?,
            Expr::Binary(op, l, r) => match op {
                _ if op.is_comparison() => Synth::Known(Prim::Bool),
                Op::And | Op::Or => Synth::Known(Prim::Bool),
                _ if op.is_shift() => self.synth(l)?,
                Op::Pow | Op::WrapPow => self.synth(l)?,
                _ => join(op.symbol(), self.synth(l)?, self.synth(r)?)?,
            },
            Expr::If(_, t, e) => match (self.synth(t)?, self.synth(e)?) {
                (Synth::Known(a), Synth::Known(b)) if a != b => bail!(
                    "expected {} for the `else` branch, found {}",
                    a.name(),
                    b.name()
                ),
                (a, b) => join("if", a, b)?,
            },
            Expr::Block(_, e) => self.synth(e)?,
            Expr::Cast(_, ty) => Synth::Known(*ty),
            Expr::Method(m, r, a) => match m.is_rotate() {
                true => self.synth(r)?,
                false => join(m.name(), self.synth(r)?, self.synth(a)?)?,
            },
            Expr::Array(_) | Expr::Bytes(..) => bail!("{ARRAY_POSITION}"),
        })
    }

    /// Checks the closure's result, returning the number of items it gives:
    /// the length of an array, or 1.
    fn check_result(&self, e: &Expr) -> Result<u32> {
        let item = self.item;
        match e {
            Expr::Array(elements) => {
                for e in elements {
                    self.check(e, item)?;
                }
                Ok(elements.len() as u32)
            }
            Expr::Bytes(value, big) => {
                let name = bytes_name(*big);
                if item != Prim::U8 {
                    bail!(
                        "`{name}` gives bytes, the stream's items must be u8, not {}",
                        item.name()
                    );
                }
                let source = self.synth(value)?.resolve();
                if source == Prim::Bool {
                    bail!("`{name}` isn't supported for bool");
                }
                self.check(value, source)?;
                Ok(source.size())
            }
            Expr::Block(stmts, e) => {
                self.check_stmts(stmts)?;
                self.check_result(e)
            }
            Expr::If(c, t, e) => {
                self.check_condition(c, "an `if`")?;
                let (a, b) = (self.check_result(t)?, self.check_result(e)?);
                if a != b {
                    bail!(
                        "the `if` branches give {a} and {b} items, they must give the same \
                         number"
                    );
                }
                Ok(a)
            }
            _ => {
                if let Synth::Known(ty) = self.synth(e)?
                    && ty != item
                {
                    bail!(
                        "expected the closure to return {}, found {}",
                        item.name(),
                        ty.name()
                    );
                }
                self.check(e, item)?;
                Ok(1)
            }
        }
    }

    fn check_stmts(&self, stmts: &[Stmt]) -> Result<()> {
        for stmt in stmts {
            match stmt {
                Stmt::Let(l) => {
                    let ty = match l.local {
                        Some(id) => self.locals[id as usize],
                        None => l.ty.unwrap_or(self.synth(&l.init)?.resolve()),
                    };
                    self.check(&l.init, ty)?;
                }
                Stmt::Assign { local, value } => {
                    let ty = self.locals[*local as usize];
                    if let Synth::Known(actual) = self.synth(value)?
                        && actual != ty
                    {
                        bail!(
                            "expected {} for an assignment, found {}",
                            ty.name(),
                            actual.name()
                        );
                    }
                    self.check(value, ty)?;
                }
                Stmt::While { cond, body } => {
                    self.check_condition(cond, "a `while`")?;
                    self.check_stmts(body)?;
                }
                Stmt::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    self.check_condition(cond, "an `if`")?;
                    self.check_stmts(then)?;
                    self.check_stmts(otherwise)?;
                }
            }
        }
        Ok(())
    }

    fn check_condition(&self, c: &Expr, what: &str) -> Result<()> {
        if let Synth::Known(actual) = self.synth(c)?
            && actual != Prim::Bool
        {
            bail!(
                "expected bool for {what} condition, found {}",
                actual.name()
            );
        }
        self.check(c, Prim::Bool)
    }

    /// The type of an exponent, its own type, or for a literal, `u32` for an
    /// integer base as Rust's `pow`, and `i32` for a float base as `powi`.
    fn exponent_type(&self, r: &Expr, base: Prim) -> Result<Prim> {
        Ok(match self.synth(r)? {
            Synth::Known(exponent) => exponent,
            _ if base.is_float() => Prim::S32,
            _ => Prim::U32,
        })
    }

    /// The operands' type for a comparison.
    fn comparison_type(&self, l: &Expr, r: &Expr) -> Result<Prim> {
        Ok(join("comparison", self.synth(l)?, self.synth(r)?)?.resolve())
    }

    /// The type of a shift amount, its own type, or the shifted value's type
    /// for a literal.
    fn amount_type(&self, r: &Expr, ty: Prim) -> Result<Prim> {
        Ok(match self.synth(r)? {
            Synth::Known(amount) => amount,
            _ => ty,
        })
    }

    /// Checks `e` has type `ty`.
    fn check(&self, e: &Expr, ty: Prim) -> Result<()> {
        let found = |actual: Prim| -> Result<()> {
            if actual != ty {
                bail!("expected {}, found {}", ty.name(), actual.name());
            }
            Ok(())
        };
        match e {
            Expr::Int(v) => {
                if ty == Prim::Bool {
                    bail!("expected bool, found integer {v}");
                }
                if ty.is_int() {
                    let (min, max) = ty.range();
                    if *v < min || *v > max {
                        bail!("literal {v} is out of range for {}", ty.name());
                    }
                }
            }
            Expr::Float(v) => {
                if !ty.is_float() {
                    bail!("expected {}, found float {v}", ty.name());
                }
                if ty == Prim::F32 && !(*v as f32).is_finite() {
                    bail!("literal {v} is out of range for f32");
                }
            }
            Expr::Bool(_) => found(Prim::Bool)?,
            Expr::Param(_) => found(self.item)?,
            Expr::Local(id) => found(self.locals[*id as usize])?,
            Expr::SeedName(..) => unreachable!("resolved before types are checked"),
            Expr::Seed(field) => found(self.seed(*field)?)?,
            Expr::Index => found(Prim::U64)?,
            Expr::Neg(e) => {
                if !ty.signed() && !ty.is_float() {
                    bail!("`-` can't negate {}", ty.name());
                }
                self.check(e, ty)?;
            }
            Expr::WrapNeg(e) => {
                if !ty.is_int() {
                    bail!("`-%` isn't supported for {}", ty.name());
                }
                self.check(e, ty)?;
            }
            Expr::Not(e) => {
                if ty.is_float() {
                    bail!("`!` isn't supported for {}", ty.name());
                }
                self.check(e, ty)?;
            }
            Expr::Binary(op, l, r) => {
                let symbol = op.symbol();
                match op {
                    _ if op.is_comparison() => {
                        found(Prim::Bool)?;
                        let operands = self.comparison_type(l, r)?;
                        self.check(l, operands)?;
                        self.check(r, operands)?;
                    }
                    Op::And | Op::Or => {
                        found(Prim::Bool)?;
                        for e in [l, r] {
                            if let Synth::Known(t) = self.synth(e)?
                                && t != Prim::Bool
                            {
                                bail!("`{symbol}` expects bool operands");
                            }
                            self.check(e, Prim::Bool)?;
                        }
                    }
                    Op::Pow | Op::WrapPow => {
                        if ty == Prim::Bool {
                            bail!("`{symbol}` expects numeric operands");
                        }
                        if *op == Op::WrapPow && ty.is_float() {
                            bail!("`{symbol}` isn't supported for {}", ty.name());
                        }
                        self.check(l, ty)?;
                        let exponent = self.exponent_type(r, ty)?;
                        if !exponent.is_int() {
                            bail!(
                                "the exponent of `{symbol}` must be an integer, found {}",
                                exponent.name()
                            );
                        }
                        self.check(r, exponent)?;
                    }
                    _ if op.is_shift() => {
                        if ty == Prim::Bool {
                            bail!("`{symbol}` expects numeric operands");
                        }
                        if ty.is_float() {
                            bail!("`{symbol}` isn't supported for {}", ty.name());
                        }
                        self.check(l, ty)?;
                        let amount = self.amount_type(r, ty)?;
                        if !amount.is_int() {
                            bail!("a shift amount must be an integer, found {}", amount.name());
                        }
                        self.check(r, amount)?;
                    }
                    _ if op.is_wrapping() => {
                        if !ty.is_int() {
                            bail!("`{symbol}` isn't supported for {}", ty.name());
                        }
                        self.check(l, ty)?;
                        self.check(r, ty)?;
                    }
                    Op::BitAnd | Op::BitOr | Op::BitXor => {
                        if ty.is_float() {
                            bail!("`{symbol}` isn't supported for {}", ty.name());
                        }
                        self.check(l, ty)?;
                        self.check(r, ty)?;
                    }
                    _ => {
                        if ty == Prim::Bool {
                            bail!("`{symbol}` expects numeric operands");
                        }
                        if ty.is_float() && *op == Op::Rem {
                            bail!("`{symbol}` isn't supported for {}", ty.name());
                        }
                        self.check(l, ty)?;
                        self.check(r, ty)?;
                    }
                }
            }
            Expr::If(c, t, e) => {
                self.check_condition(c, "an `if`")?;
                self.check(t, ty)?;
                self.check(e, ty)?;
            }
            Expr::Block(stmts, e) => {
                self.check_stmts(stmts)?;
                self.check(e, ty)?;
            }
            Expr::Array(_) | Expr::Bytes(..) => bail!("{ARRAY_POSITION}"),
            Expr::Cast(e, target) => {
                found(*target)?;
                if *target == Prim::Bool {
                    bail!("can't cast to bool, compare instead, e.g. `x != 0`");
                }
                let source = self.synth(e)?.resolve();
                if source == Prim::Bool && !target.is_int() {
                    bail!("can't cast bool to {}", target.name());
                }
                self.check(e, source)?;
            }
            Expr::Method(m, r, a) => {
                if !ty.is_int() {
                    bail!("`{}` isn't supported for {}", m.name(), ty.name());
                }
                self.check(r, ty)?;
                self.check(a, if m.is_rotate() { Prim::U32 } else { ty })?;
            }
        }
        Ok(())
    }

    /// `generate(state, ptr) -> ok` reads the previous items from `ptr`, and
    /// `$init` and `$i` from the task's `state`, storing the next item after
    /// the previous items, returning 0 when it can't be generated.
    pub fn function(&self) -> Function {
        use ValType::*;
        let mut locals = vec![
            (2, I32),
            (3, I64),
            (1, F32),
            (1, F64),
            (3, I32),
            (3, I64),
            (2, F32),
            (2, F64),
        ];
        locals.extend(self.locals.iter().map(|ty| (1, ty.val_type())));
        let mut f = Function::new(locals);
        let i = &mut f.instructions();
        i.block(BlockType::Empty);
        self.emit_result(i, &self.body, 0);
        i.i32_const(1).return_().end().i32_const(0).end();
        f
    }

    /// Emits the closure's result, storing its items after the previous items.
    fn emit_result(&self, i: &mut InstructionSink, e: &Expr, fail: u32) {
        let item = self.item;
        let offset = self.params * item.size();
        match e {
            Expr::Array(elements) => {
                for (index, e) in elements.iter().enumerate() {
                    i.local_get(PTR);
                    self.emit(i, e, item, fail);
                    item.store(i, offset + index as u32 * item.size());
                }
            }
            Expr::Bytes(value, big) => {
                let source = self.synth(value).map_or(Prim::U64, Synth::resolve);
                let wide = source.size() == 8;
                self.emit(i, value, source, fail);
                match source {
                    Prim::F32 => {
                        i.i32_reinterpret_f32();
                    }
                    Prim::F64 => {
                        i.i64_reinterpret_f64();
                    }
                    _ => {}
                }
                let temp = if wide { A64 } else { A32 };
                i.local_set(temp);
                let byte = |offset: u32| MemArg {
                    offset: offset.into(),
                    align: 0,
                    memory_index: 0,
                };
                if *big {
                    // most significant byte first
                    for index in 0..source.size() {
                        let shift = (source.size() - 1 - index) * 8;
                        i.local_get(PTR).local_get(temp);
                        match wide {
                            true => i
                                .i64_const(shift as i64)
                                .i64_shr_u()
                                .i64_store8(byte(offset + index)),
                            false => i
                                .i32_const(shift as i32)
                                .i32_shr_u()
                                .i32_store8(byte(offset + index)),
                        };
                    }
                } else {
                    // wasm stores little endian
                    i.local_get(PTR).local_get(temp);
                    match source.size() {
                        1 => i.i32_store8(byte(offset)),
                        2 => i.i32_store16(byte(offset)),
                        4 => i.i32_store(byte(offset)),
                        _ => i.i64_store(byte(offset)),
                    };
                }
            }
            Expr::Block(stmts, e) => {
                self.emit_stmts(i, stmts, fail);
                self.emit_result(i, e, fail);
            }
            Expr::If(c, t, e) => {
                self.emit(i, c, Prim::Bool, fail);
                i.if_(BlockType::Empty);
                self.emit_result(i, t, fail + 1);
                i.else_();
                self.emit_result(i, e, fail + 1);
                i.end();
            }
            _ => {
                i.local_get(PTR);
                self.emit(i, e, item, fail);
                item.store(i, offset);
            }
        }
    }

    fn emit_stmts(&self, i: &mut InstructionSink, stmts: &[Stmt], fail: u32) {
        for stmt in stmts {
            match stmt {
                Stmt::Let(l) => {
                    let ty = match l.local {
                        Some(id) => self.locals[id as usize],
                        None => l.ty.unwrap_or_else(|| {
                            self.synth(&l.init).map_or(Prim::S32, Synth::resolve)
                        }),
                    };
                    self.emit(i, &l.init, ty, fail);
                    match l.local {
                        Some(id) => i.local_set(FIRST_LOCAL + id),
                        None => i.drop(),
                    };
                }
                Stmt::Assign { local, value } => {
                    self.emit(i, value, self.locals[*local as usize], fail);
                    i.local_set(FIRST_LOCAL + local);
                }
                // the loop's block and loop are 2 more blocks to break out of
                Stmt::While { cond, body } => {
                    i.block(BlockType::Empty).loop_(BlockType::Empty);
                    self.emit(i, cond, Prim::Bool, fail + 2);
                    i.i32_eqz().br_if(1);
                    self.emit_stmts(i, body, fail + 2);
                    i.br(0).end().end();
                }
                Stmt::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    self.emit(i, cond, Prim::Bool, fail);
                    i.if_(BlockType::Empty);
                    self.emit_stmts(i, then, fail + 1);
                    if !otherwise.is_empty() {
                        i.else_();
                        self.emit_stmts(i, otherwise, fail + 1);
                    }
                    i.end();
                }
            }
        }
    }

    /// Emits `e` as type `ty`, `fail` is the depth of the block to break to
    /// when an item can't be generated.
    fn emit(&self, i: &mut InstructionSink, e: &Expr, ty: Prim, fail: u32) {
        match e {
            Expr::Int(v) => match ty.val_type() {
                ValType::I64 => {
                    i.i64_const(*v as i64);
                }
                ValType::F32 => {
                    i.f32_const((*v as f32).into());
                }
                ValType::F64 => {
                    i.f64_const((*v as f64).into());
                }
                _ => {
                    i.i32_const(*v as i32);
                }
            },
            Expr::Float(v) => match ty {
                Prim::F32 => {
                    i.f32_const((*v as f32).into());
                }
                _ => {
                    i.f64_const((*v).into());
                }
            },
            Expr::Bool(v) => {
                i.i32_const(*v as i32);
            }
            Expr::Param(index) => {
                i.local_get(PTR);
                ty.load(i, index * ty.size());
            }
            Expr::Local(id) => {
                i.local_get(FIRST_LOCAL + id);
            }
            Expr::SeedName(..) => unreachable!("resolved before types are checked"),
            Expr::Seed(field) => {
                i.local_get(STATE);
                ty.load(i, SEEDS + field * 8);
            }
            Expr::Index => {
                i.local_get(STATE).i64_load(MemArg {
                    offset: INDEX.into(),
                    align: 3,
                    memory_index: 0,
                });
            }
            Expr::Neg(e) => {
                if ty.is_float() {
                    self.emit(i, e, ty, fail);
                    match ty {
                        Prim::F32 => i.f32_neg(),
                        _ => i.f64_neg(),
                    };
                } else if ty.narrow() {
                    i.i64_const(0);
                    self.emit(i, e, ty, fail);
                    i.i64_extend_i32_s().i64_sub();
                    range(i, ty, fail);
                } else {
                    // only the minimum can't be negated
                    self.emit(i, e, ty, fail);
                    i.local_tee(A64)
                        .i64_const(i64::MIN)
                        .i64_eq()
                        .br_if(fail)
                        .i64_const(0)
                        .local_get(A64)
                        .i64_sub();
                }
            }
            Expr::WrapNeg(e) => {
                match ty.wide() {
                    true => i.i64_const(0),
                    false => i.i32_const(0),
                };
                self.emit(i, e, ty, fail);
                wrapping_op(i, Op::WrapSub, ty, fail);
            }
            Expr::Not(e) => {
                self.emit(i, e, ty, fail);
                if ty == Prim::Bool {
                    i.i32_eqz();
                } else if ty.narrow() && !ty.signed() {
                    i.i32_const(ty.range().1 as i32).i32_xor();
                } else if ty.narrow() {
                    i.i32_const(-1).i32_xor();
                } else {
                    i.i64_const(-1).i64_xor();
                }
            }
            Expr::If(c, t, e) => {
                self.emit(i, c, Prim::Bool, fail);
                i.if_(BlockType::Result(ty.val_type()));
                self.emit(i, t, ty, fail + 1);
                i.else_();
                self.emit(i, e, ty, fail + 1);
                i.end();
            }
            Expr::Block(stmts, e) => {
                self.emit_stmts(i, stmts, fail);
                self.emit(i, e, ty, fail);
            }
            Expr::Array(_) | Expr::Bytes(..) => unreachable!("checked as the result"),
            Expr::Cast(e, target) => {
                let source = self.synth(e).map_or(*target, Synth::resolve);
                self.emit(i, e, source, fail);
                convert(i, source, *target);
            }
            Expr::Method(m, r, a) => {
                self.emit(i, r, ty, fail);
                self.emit(i, a, if m.is_rotate() { Prim::U32 } else { ty }, fail);
                method(i, *m, ty);
            }
            Expr::Binary(Op::And, l, r) => {
                self.emit(i, l, Prim::Bool, fail);
                i.if_(BlockType::Result(ValType::I32));
                self.emit(i, r, Prim::Bool, fail + 1);
                i.else_().i32_const(0).end();
            }
            Expr::Binary(Op::Or, l, r) => {
                self.emit(i, l, Prim::Bool, fail);
                i.if_(BlockType::Result(ValType::I32)).i32_const(1).else_();
                self.emit(i, r, Prim::Bool, fail + 1);
                i.end();
            }
            Expr::Binary(op, l, r) if op.is_comparison() => {
                let operands = self.comparison_type(l, r).unwrap_or(ty);
                self.emit(i, l, operands, fail);
                self.emit(i, r, operands, fail);
                match operands {
                    Prim::Bool => bool_op(i, *op),
                    _ => compare(i, *op, operands),
                }
            }
            Expr::Binary(op @ (Op::Pow | Op::WrapPow), l, r) => {
                let exponent = self.exponent_type(r, ty).unwrap_or(Prim::U32);
                self.emit(i, l, ty, fail);
                self.emit(i, r, exponent, fail);
                pow(i, *op, ty, exponent, fail);
            }
            Expr::Binary(op, l, r) if op.is_shift() => {
                let amount = self.amount_type(r, ty).unwrap_or(ty);
                self.emit(i, l, ty, fail);
                self.emit(i, r, amount, fail);
                shift(i, *op, ty, amount, fail);
            }
            Expr::Binary(op, l, r) => {
                self.emit(i, l, ty, fail);
                self.emit(i, r, ty, fail);
                match ty {
                    _ if op.is_wrapping() => wrapping_op(i, *op, ty, fail),
                    Prim::Bool => bool_op(i, *op),
                    _ if ty.is_float() => float_op(i, *op, ty, fail),
                    _ if ty.narrow() => narrow_op(i, *op, ty, fail),
                    _ => wide_op(i, *op, ty, fail),
                }
            }
        }
    }
}

const ARRAY_POSITION: &str =
    "an array can only be the closure's result, e.g. `|| [a, b]` or `|| x.to_le_bytes()`";

fn bytes_name(big: bool) -> &'static str {
    match big {
        true => "to_be_bytes",
        false => "to_le_bytes",
    }
}

/// The joined type of two operands that must have the same type.
fn join(what: &str, a: Synth, b: Synth) -> Result<Synth> {
    Ok(match (a, b) {
        (Synth::Known(a), Synth::Known(b)) if a != b => bail!(
            "mismatched types for `{what}`, {} and {}",
            a.name(),
            b.name()
        ),
        (Synth::Known(a), _) | (_, Synth::Known(a)) => Synth::Known(a),
        (Synth::Float, _) | (_, Synth::Float) => Synth::Float,
        _ => Synth::Int,
    })
}

/// Re-extends the low bits of an i32 holding a narrow integer, after an
/// operation that may have set the bits above the type's width.
fn canonical(i: &mut InstructionSink, ty: Prim) {
    let unused = 32 - ty.bits() as i32;
    if !ty.narrow() || unused == 0 {
        return;
    }
    if ty.signed() {
        i.i32_const(unused).i32_shl().i32_const(unused).i32_shr_s();
    } else {
        i.i32_const(ty.range().1 as i32).i32_and();
    }
}

/// Checks the i64 result of a narrow integer operation fits `ty`, wrapping it
/// to an i32.
fn range(i: &mut InstructionSink, ty: Prim, fail: u32) {
    let (min, max) = ty.range();
    i.local_tee(R64);
    if ty.signed() {
        i.i64_const(min as i64)
            .i64_lt_s()
            .br_if(fail)
            .local_get(R64)
            .i64_const(max as i64)
            .i64_gt_s()
            .br_if(fail);
    } else {
        i.i64_const(max as i64).i64_gt_u().br_if(fail);
    }
    i.local_get(R64).i32_wrap_i64();
}

/// An arithmetic or bitwise operation on integers of at most 32 bits, checked
/// by computing in 64 bits.
fn narrow_op(i: &mut InstructionSink, op: Op, ty: Prim, fail: u32) {
    let (min, _) = ty.range();
    i.local_set(B32).local_set(A32);
    let extend = |i: &mut InstructionSink, local| {
        i.local_get(local);
        match ty.signed() {
            true => i.i64_extend_i32_s(),
            false => i.i64_extend_i32_u(),
        };
    };
    match op {
        Op::BitAnd | Op::BitOr | Op::BitXor => {
            i.local_get(A32).local_get(B32);
            match op {
                Op::BitAnd => i.i32_and(),
                Op::BitOr => i.i32_or(),
                _ => i.i32_xor(),
            };
        }
        _ => {
            if matches!(op, Op::Div | Op::Rem) {
                i.local_get(B32).i32_eqz().br_if(fail);
                if ty.signed() {
                    // Rust's checked `%` also fails for the minimum and -1
                    i.local_get(A32)
                        .i32_const(min as i32)
                        .i32_eq()
                        .local_get(B32)
                        .i32_const(-1)
                        .i32_eq()
                        .i32_and()
                        .br_if(fail);
                }
            }
            extend(i, A32);
            extend(i, B32);
            match (op, ty.signed()) {
                (Op::Add, _) => i.i64_add(),
                (Op::Sub, _) => i.i64_sub(),
                (Op::Mul, _) => i.i64_mul(),
                (Op::Div, true) => i.i64_div_s(),
                (Op::Div, false) => i.i64_div_u(),
                (Op::Rem, true) => i.i64_rem_s(),
                _ => i.i64_rem_u(),
            };
            range(i, ty, fail);
        }
    }
}

/// An arithmetic or bitwise operation on 64-bit integers, checked as Rust's
/// checked arithmetic.
fn wide_op(i: &mut InstructionSink, op: Op, ty: Prim, fail: u32) {
    let signed = ty.signed();
    i.local_set(B64).local_set(A64);
    match op {
        Op::Add | Op::Sub => {
            i.local_get(A64).local_get(B64);
            match op {
                Op::Add => i.i64_add(),
                _ => i.i64_sub(),
            };
            i.local_set(R64);
            match (op, signed) {
                // the sum wrapped below either operand
                (Op::Add, false) => i.local_get(R64).local_get(A64).i64_lt_u(),
                (Op::Sub, false) => i.local_get(A64).local_get(B64).i64_lt_u(),
                // the operands' signs agree and the result's sign differs
                (Op::Add, true) => i
                    .local_get(A64)
                    .local_get(R64)
                    .i64_xor()
                    .local_get(B64)
                    .local_get(R64)
                    .i64_xor()
                    .i64_and()
                    .i64_const(0)
                    .i64_lt_s(),
                // the operands' signs differ and the result's sign differs from a
                _ => i
                    .local_get(A64)
                    .local_get(B64)
                    .i64_xor()
                    .local_get(A64)
                    .local_get(R64)
                    .i64_xor()
                    .i64_and()
                    .i64_const(0)
                    .i64_lt_s(),
            };
            i.br_if(fail).local_get(R64);
        }
        Op::Mul => {
            if signed {
                min_by_minus_one(i, B64, A64, fail);
            }
            i.local_get(A64).local_get(B64).i64_mul().local_set(R64);
            // the product divided by a non-zero operand gives back the other
            i.local_get(A64)
                .i64_const(0)
                .i64_ne()
                .if_(BlockType::Empty)
                .local_get(R64)
                .local_get(A64);
            match signed {
                true => i.i64_div_s(),
                false => i.i64_div_u(),
            };
            i.local_get(B64)
                .i64_ne()
                .br_if(fail + 1)
                .end()
                .local_get(R64);
        }
        Op::Div | Op::Rem => {
            i.local_get(B64).i64_eqz().br_if(fail);
            if signed {
                min_by_minus_one(i, A64, B64, fail);
            }
            i.local_get(A64).local_get(B64);
            match (op, signed) {
                (Op::Div, true) => i.i64_div_s(),
                (Op::Div, false) => i.i64_div_u(),
                (_, true) => i.i64_rem_s(),
                _ => i.i64_rem_u(),
            };
        }
        _ => {
            i.local_get(A64).local_get(B64);
            match op {
                Op::BitAnd => i.i64_and(),
                Op::BitOr => i.i64_or(),
                _ => i.i64_xor(),
            };
        }
    }
}

/// `a << b` or `a >> b`, where `a` is a `ty` and the amount `b` is an
/// `amount`. The amount must be less than `ty`'s bits, read as unsigned. Bits
/// shifted past the type's width are dropped, as in Rust.
fn shift(i: &mut InstructionSink, op: Op, ty: Prim, amount: Prim, fail: u32) {
    let (a, b) = (
        if ty.wide() { A64 } else { A32 },
        if amount.wide() { B64 } else { B32 },
    );
    let wrapping = matches!(op, Op::WrapShl | Op::WrapShr);
    i.local_set(b).local_set(a);
    if !wrapping {
        i.local_get(b);
        match amount.wide() {
            true => i.i64_const(ty.bits() as i64).i64_ge_u(),
            false => i.i32_const(ty.bits() as i32).i32_ge_u(),
        };
        i.br_if(fail);
    }
    i.local_get(a).local_get(b);
    match (ty.wide(), amount.wide()) {
        (true, false) => {
            i.i64_extend_i32_u();
        }
        (false, true) => {
            i.i32_wrap_i64();
        }
        _ => {}
    }
    // a wrapping shift takes the amount modulo the type's bits
    if wrapping {
        match ty.wide() {
            true => i.i64_const(ty.bits() as i64 - 1).i64_and(),
            false => i.i32_const(ty.bits() as i32 - 1).i32_and(),
        };
    }
    match (op, ty.wide(), ty.signed()) {
        (Op::Shl | Op::WrapShl, true, _) => {
            i.i64_shl();
        }
        (Op::Shl | Op::WrapShl, false, _) => {
            i.i32_shl();
            canonical(i, ty);
        }
        (_, true, true) => {
            i.i64_shr_s();
        }
        (_, true, false) => {
            i.i64_shr_u();
        }
        (_, false, true) => {
            i.i32_shr_s();
        }
        _ => {
            i.i32_shr_u();
        }
    }
}

/// `base ** exponent`, by squaring: for each bit of the exponent, lowest
/// first, the result is multiplied by the base when the bit is set, and the
/// base squared while bits are left. The exponent is any integer, counted in
/// 64 bits. Integers multiply as `*`, failing on overflow, or as `*%` for
/// `**%`, and fail for a negative exponent. A float's negative exponent gives
/// the reciprocal, and its result must be finite.
fn pow(i: &mut InstructionSink, op: Op, ty: Prim, exponent: Prim, fail: u32) {
    let (result, base) = match ty {
        Prim::F32 => (POW_RESULT_F32, POW_BASE_F32),
        Prim::F64 => (POW_RESULT_F64, POW_BASE_F64),
        _ if ty.wide() => (POW_RESULT_64, POW_BASE_64),
        _ => (POW_RESULT_32, POW_BASE_32),
    };
    // the exponent, extended to 64 bits
    if !exponent.wide() {
        match exponent.signed() {
            true => i.i64_extend_i32_s(),
            false => i.i64_extend_i32_u(),
        };
    }
    i.local_set(POW_EXPONENT).local_set(base);
    i.i32_const(0).local_set(POW_NEGATIVE);
    if exponent.signed() {
        i.local_get(POW_EXPONENT).i64_const(0).i64_lt_s();
        match ty.is_float() {
            // an integer's negative power isn't an integer
            false => {
                i.br_if(fail);
            }
            // a float's is the reciprocal, of its magnitude read as unsigned
            true => {
                i.local_tee(POW_NEGATIVE)
                    .if_(BlockType::Empty)
                    .i64_const(0)
                    .local_get(POW_EXPONENT)
                    .i64_sub()
                    .local_set(POW_EXPONENT)
                    .end();
            }
        }
    }
    match ty {
        Prim::F32 => i.f32_const(1.0f32.into()),
        Prim::F64 => i.f64_const(1.0f64.into()),
        _ if ty.wide() => i.i64_const(1),
        _ => i.i32_const(1),
    };
    i.local_set(result);
    // multiplies the two values on the stack, inside the loop's block, loop
    // and an if, 3 blocks deeper than `fail`
    let multiply = |i: &mut InstructionSink| match (op, ty) {
        (_, Prim::F32) => {
            i.f32_mul();
        }
        (_, Prim::F64) => {
            i.f64_mul();
        }
        (Op::WrapPow, _) => wrapping_op(i, Op::WrapMul, ty, fail + 3),
        _ if ty.narrow() => narrow_op(i, Op::Mul, ty, fail + 3),
        _ => wide_op(i, Op::Mul, ty, fail + 3),
    };
    i.block(BlockType::Empty)
        .loop_(BlockType::Empty)
        .local_get(POW_EXPONENT)
        .i64_eqz()
        .br_if(1)
        .local_get(POW_EXPONENT)
        .i64_const(1)
        .i64_and()
        .i32_wrap_i64()
        .if_(BlockType::Empty)
        .local_get(result)
        .local_get(base);
    multiply(i);
    i.local_set(result)
        .end()
        .local_get(POW_EXPONENT)
        .i64_const(1)
        .i64_shr_u()
        .local_tee(POW_EXPONENT)
        .i64_const(0)
        .i64_ne()
        .if_(BlockType::Empty)
        .local_get(base)
        .local_get(base);
    multiply(i);
    i.local_set(base).end().br(0).end().end();
    if !ty.is_float() {
        i.local_get(result);
        return;
    }
    i.local_get(POW_NEGATIVE)
        .if_(BlockType::Result(ty.val_type()));
    match ty {
        Prim::F32 => i.f32_const(1.0f32.into()).local_get(result).f32_div(),
        _ => i.f64_const(1.0f64.into()).local_get(result).f64_div(),
    };
    i.else_().local_get(result).end();
    // the result must be finite, NaN isn't less than infinity either
    match ty {
        Prim::F32 => i
            .local_tee(F32)
            .f32_abs()
            .f32_const(f32::INFINITY.into())
            .f32_lt()
            .i32_eqz()
            .br_if(fail)
            .local_get(F32),
        _ => i
            .local_tee(F64)
            .f64_abs()
            .f64_const(f64::INFINITY.into())
            .f64_lt()
            .i32_eqz()
            .br_if(fail)
            .local_get(F64),
    };
}

/// Arithmetic on floats, the result must be finite.
fn float_op(i: &mut InstructionSink, op: Op, ty: Prim, fail: u32) {
    match (ty, op) {
        (Prim::F32, Op::Add) => i.f32_add(),
        (Prim::F32, Op::Sub) => i.f32_sub(),
        (Prim::F32, Op::Mul) => i.f32_mul(),
        (Prim::F32, _) => i.f32_div(),
        (_, Op::Add) => i.f64_add(),
        (_, Op::Sub) => i.f64_sub(),
        (_, Op::Mul) => i.f64_mul(),
        _ => i.f64_div(),
    };
    // NaN isn't less than infinity either
    match ty {
        Prim::F32 => i
            .local_tee(F32)
            .f32_abs()
            .f32_const(f32::INFINITY.into())
            .f32_lt()
            .i32_eqz()
            .br_if(fail)
            .local_get(F32),
        _ => i
            .local_tee(F64)
            .f64_abs()
            .f64_const(f64::INFINITY.into())
            .f64_lt()
            .i32_eqz()
            .br_if(fail)
            .local_get(F64),
    };
}

/// A method on integers of type `ty`, wrapping arithmetic never fails, and a
/// rotation's `u32` amount is taken modulo `ty`'s bits.
fn method(i: &mut InstructionSink, m: Method, ty: Prim) {
    match (m, ty.wide()) {
        (Method::WrappingAdd, _) => wrapping_op(i, Op::WrapAdd, ty, 0),
        (Method::WrappingSub, _) => wrapping_op(i, Op::WrapSub, ty, 0),
        (Method::WrappingMul, _) => wrapping_op(i, Op::WrapMul, ty, 0),
        (Method::RotateLeft, true) => {
            i.i64_extend_i32_u().i64_rotl();
        }
        (Method::RotateRight, true) => {
            i.i64_extend_i32_u().i64_rotr();
        }
        (_, false) if ty.bits() == 32 => {
            match m {
                Method::RotateLeft => i.i32_rotl(),
                _ => i.i32_rotr(),
            };
        }
        (_, false) => {
            // rotate the type's bits within an i32, then re-extend them
            let bits = ty.bits() as i32;
            // a left rotation shifts the bits left, then the overflow right
            let left = m == Method::RotateLeft;
            let shift = |i: &mut InstructionSink, left: bool| {
                match left {
                    true => i.i32_shl(),
                    false => i.i32_shr_u(),
                };
            };
            i.i32_const(bits - 1)
                .i32_and()
                .local_set(B32)
                .i32_const((1 << bits) - 1)
                .i32_and()
                .local_set(A32)
                .local_get(A32)
                .local_get(B32);
            shift(i, left);
            i.local_get(A32)
                .i32_const(bits)
                .local_get(B32)
                .i32_sub()
                .i32_const(bits - 1)
                .i32_and();
            shift(i, !left);
            i.i32_or();
            if ty.signed() {
                canonical(i, ty);
            } else {
                i.i32_const((1 << bits) - 1).i32_and();
            }
        }
    }
}

/// Wrapping arithmetic on integers, as Rust's `wrapping_*` methods: the
/// result wraps around the type's range. Division and remainder still fail
/// for a zero divisor.
fn wrapping_op(i: &mut InstructionSink, op: Op, ty: Prim, fail: u32) {
    match (op, ty.wide()) {
        (Op::WrapAdd, true) => {
            i.i64_add();
        }
        (Op::WrapSub, true) => {
            i.i64_sub();
        }
        (Op::WrapMul, true) => {
            i.i64_mul();
        }
        (Op::WrapAdd, false) => {
            i.i32_add();
            canonical(i, ty);
        }
        (Op::WrapSub, false) => {
            i.i32_sub();
            canonical(i, ty);
        }
        (Op::WrapMul, false) => {
            i.i32_mul();
            canonical(i, ty);
        }
        // narrow integers divide in 64 bits, where the minimum divided by -1
        // doesn't overflow, then wrap
        (_, false) => {
            i.local_set(B32)
                .local_set(A32)
                .local_get(B32)
                .i32_eqz()
                .br_if(fail);
            for local in [A32, B32] {
                i.local_get(local);
                match ty.signed() {
                    true => i.i64_extend_i32_s(),
                    false => i.i64_extend_i32_u(),
                };
            }
            match (op, ty.signed()) {
                (Op::WrapDiv, true) => i.i64_div_s(),
                (Op::WrapDiv, false) => i.i64_div_u(),
                (_, true) => i.i64_rem_s(),
                _ => i.i64_rem_u(),
            };
            i.i32_wrap_i64();
            canonical(i, ty);
        }
        (_, true) => {
            i.local_set(B64)
                .local_set(A64)
                .local_get(B64)
                .i64_eqz()
                .br_if(fail);
            match (op, ty.signed()) {
                // the minimum divided by -1 wraps to the minimum, negating
                (Op::WrapDiv, true) => {
                    i.local_get(B64)
                        .i64_const(-1)
                        .i64_eq()
                        .if_(BlockType::Result(ValType::I64))
                        .i64_const(0)
                        .local_get(A64)
                        .i64_sub()
                        .else_()
                        .local_get(A64)
                        .local_get(B64)
                        .i64_div_s()
                        .end();
                }
                (Op::WrapDiv, false) => {
                    i.local_get(A64).local_get(B64).i64_div_u();
                }
                // wasm's remainder of the minimum and -1 is 0, as Rust's wrapping_rem
                (_, true) => {
                    i.local_get(A64).local_get(B64).i64_rem_s();
                }
                _ => {
                    i.local_get(A64).local_get(B64).i64_rem_u();
                }
            }
        }
    }
}

/// Converts a `source` value to `target`, as Rust's `as`: integers are
/// truncated, or sign or zero extended, floats are rounded, and floats
/// converted to integers saturate, with NaN becoming zero.
fn convert(i: &mut InstructionSink, source: Prim, target: Prim) {
    use Prim::*;
    if source == target {
        return;
    }
    match (source.is_float(), target.is_float()) {
        // integers, including bool, to integers
        (false, false) => match (source.wide(), target.wide()) {
            (false, true) => {
                match source.signed() {
                    true => i.i64_extend_i32_s(),
                    false => i.i64_extend_i32_u(),
                };
            }
            (true, false) => {
                i.i32_wrap_i64();
                canonical(i, target);
            }
            (false, false) => canonical(i, target),
            (true, true) => {}
        },
        (false, true) => {
            match (source.wide(), source.signed(), target) {
                (false, true, F32) => i.f32_convert_i32_s(),
                (false, false, F32) => i.f32_convert_i32_u(),
                (true, true, F32) => i.f32_convert_i64_s(),
                (true, false, F32) => i.f32_convert_i64_u(),
                (false, true, _) => i.f64_convert_i32_s(),
                (false, false, _) => i.f64_convert_i32_u(),
                (true, true, _) => i.f64_convert_i64_s(),
                (true, false, _) => i.f64_convert_i64_u(),
            };
        }
        (true, true) => {
            match target {
                F32 => i.f32_demote_f64(),
                _ => i.f64_promote_f32(),
            };
        }
        (true, false) => {
            match (source, target.wide(), target.signed()) {
                (F32, true, true) => i.i64_trunc_sat_f32_s(),
                (F32, true, false) => i.i64_trunc_sat_f32_u(),
                (F32, false, true) => i.i32_trunc_sat_f32_s(),
                (F32, false, false) => i.i32_trunc_sat_f32_u(),
                (_, true, true) => i.i64_trunc_sat_f64_s(),
                (_, true, false) => i.i64_trunc_sat_f64_u(),
                (_, false, true) => i.i32_trunc_sat_f64_s(),
                _ => i.i32_trunc_sat_f64_u(),
            };
            // saturate to a narrower integer's range
            if target.narrow() && target.bits() < 32 {
                let (min, max) = target.range();
                let clamp =
                    |i: &mut InstructionSink, bound: i32, past: fn(&mut InstructionSink)| {
                        i.local_set(A32)
                            .i32_const(bound)
                            .local_get(A32)
                            .local_get(A32)
                            .i32_const(bound);
                        past(i);
                        i.select();
                    };
                match target.signed() {
                    true => {
                        clamp(i, max as i32, |i| {
                            i.i32_gt_s();
                        });
                        clamp(i, min as i32, |i| {
                            i.i32_lt_s();
                        });
                    }
                    false => clamp(i, max as i32, |i| {
                        i.i32_gt_u();
                    }),
                }
            }
        }
    }
}

/// Fails when `x` is the minimum and `y` is -1, which overflows dividing and
/// multiplying.
fn min_by_minus_one(i: &mut InstructionSink, x: u32, y: u32, fail: u32) {
    i.local_get(x)
        .i64_const(i64::MIN)
        .i64_eq()
        .local_get(y)
        .i64_const(-1)
        .i64_eq()
        .i32_and()
        .br_if(fail);
}

/// Operations on bools, as 0 or 1 i32s, `false` is less than `true`.
fn bool_op(i: &mut InstructionSink, op: Op) {
    match op {
        Op::BitAnd => i.i32_and(),
        Op::BitOr => i.i32_or(),
        Op::BitXor | Op::Ne => i.i32_ne(),
        Op::Eq => i.i32_eq(),
        Op::Lt => i.i32_lt_u(),
        Op::Le => i.i32_le_u(),
        Op::Gt => i.i32_gt_u(),
        _ => i.i32_ge_u(),
    };
}

fn compare(i: &mut InstructionSink, op: Op, ty: Prim) {
    let signed = ty.signed();
    match ty.val_type() {
        ValType::I64 => match (op, signed) {
            (Op::Eq, _) => i.i64_eq(),
            (Op::Ne, _) => i.i64_ne(),
            (Op::Lt, true) => i.i64_lt_s(),
            (Op::Lt, false) => i.i64_lt_u(),
            (Op::Le, true) => i.i64_le_s(),
            (Op::Le, false) => i.i64_le_u(),
            (Op::Gt, true) => i.i64_gt_s(),
            (Op::Gt, false) => i.i64_gt_u(),
            (_, true) => i.i64_ge_s(),
            _ => i.i64_ge_u(),
        },
        ValType::F32 => match op {
            Op::Eq => i.f32_eq(),
            Op::Ne => i.f32_ne(),
            Op::Lt => i.f32_lt(),
            Op::Le => i.f32_le(),
            Op::Gt => i.f32_gt(),
            _ => i.f32_ge(),
        },
        ValType::F64 => match op {
            Op::Eq => i.f64_eq(),
            Op::Ne => i.f64_ne(),
            Op::Lt => i.f64_lt(),
            Op::Le => i.f64_le(),
            Op::Gt => i.f64_gt(),
            _ => i.f64_ge(),
        },
        _ => match (op, signed) {
            (Op::Eq, _) => i.i32_eq(),
            (Op::Ne, _) => i.i32_ne(),
            (Op::Lt, true) => i.i32_lt_s(),
            (Op::Lt, false) => i.i32_lt_u(),
            (Op::Le, true) => i.i32_le_s(),
            (Op::Le, false) => i.i32_le_u(),
            (Op::Gt, true) => i.i32_gt_s(),
            (Op::Gt, false) => i.i32_gt_u(),
            (_, true) => i.i32_ge_s(),
            _ => i.i32_ge_u(),
        },
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tree-sitter grammar's test corpus, see grammars/constants-expression/README.md.
    const CORPUS: [&str; 9] = [
        include_str!("../../../grammars/constants-expression/test/corpus/arrays.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/blocks.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/closures.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/conditionals.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/errors.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/literals.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/loops.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/operators.txt"),
        include_str!("../../../grammars/constants-expression/test/corpus/variables.txt"),
    ];

    /// Each test of a corpus: its name, whether it's an error, and the input.
    fn cases(corpus: &str) -> Vec<(String, bool, String)> {
        let lines: Vec<&str> = corpus.lines().collect();
        let header = |line: &str| line.len() >= 3 && line.chars().all(|c| c == '=');
        let mut cases = vec![];
        let mut i = 0;
        while i < lines.len() {
            if !header(lines[i]) {
                i += 1;
                continue;
            }
            let name = lines[i + 1].to_string();
            let mut j = i + 2;
            let mut error = false;
            while !header(lines[j]) {
                error |= lines[j].trim() == ":error";
                j += 1;
            }
            let start = j + 1;
            let end = start
                + lines[start..]
                    .iter()
                    .position(|l| *l == "---")
                    .expect("---");
            cases.push((name, error, lines[start..end].join("\n").trim().to_string()));
            i = end;
        }
        cases
    }

    #[test]
    fn it_accepts_the_same_syntax_as_the_tree_sitter_grammar() {
        let cases: Vec<_> = CORPUS.iter().flat_map(|c| cases(c)).collect();
        assert!(cases.len() >= 40, "found {} cases", cases.len());
        for (name, error, input) in cases {
            let result = parse_closure(&input, &[]);
            assert_eq!(
                result.is_err(),
                error,
                "{name}: {input:?} {}",
                match &result {
                    Ok(_) => "parsed".to_string(),
                    Err(e) => format!("failed: {e}"),
                }
            );
        }
    }
}
