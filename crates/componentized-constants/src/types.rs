use anyhow::{Result, bail};
use wit_parser::{Function, FunctionKind, Handle, Resolve, Type, TypeDefKind};

/// Ensures a function can be implemented as a constant: it must be a
/// freestanding function, synchronous or async, that takes no parameters and
/// returns a value. A function generating a stream with an `@expression` may
/// take the items the stream starts from as parameters.
///
/// The result type isn't checked here; any type is allowed so long as the
/// function's value doesn't reach a type that values can't be expressed for,
/// see [`crate::values`].
pub fn check_function(func: &Function) -> Result<()> {
    crate::docs::check(&func.docs).map_err(|e| e.context(format!("function `{}`", func.name)))?;
    if !matches!(
        func.kind,
        FunctionKind::Freestanding | FunctionKind::AsyncFreestanding
    ) {
        bail!("function `{}` must be a freestanding function", func.name);
    }
    if !func.params.is_empty() && crate::docs::expression(&func.docs).is_none() {
        bail!(
            "function `{}` must not accept parameters, unless it generates a stream from \
             them with an `@expression`",
            func.name
        );
    }
    if func.result.is_none() {
        bail!("function `{}` must return a value", func.name);
    }
    Ok(())
}

/// Follows type aliases to the underlying type.
pub fn dealias(resolve: &Resolve, mut ty: Type) -> Type {
    while let Type::Id(id) = ty {
        match resolve.types[id].kind {
            TypeDefKind::Type(t) => ty = t,
            _ => break,
        }
    }
    ty
}

/// Describes a type for error messages.
pub fn describe(resolve: &Resolve, ty: Type) -> String {
    let id = match ty {
        Type::Id(id) => id,
        Type::Bool => return "`bool`".into(),
        Type::U8 => return "`u8`".into(),
        Type::U16 => return "`u16`".into(),
        Type::U32 => return "`u32`".into(),
        Type::U64 => return "`u64`".into(),
        Type::S8 => return "`s8`".into(),
        Type::S16 => return "`s16`".into(),
        Type::S32 => return "`s32`".into(),
        Type::S64 => return "`s64`".into(),
        Type::F32 => return "`f32`".into(),
        Type::F64 => return "`f64`".into(),
        Type::Char => return "`char`".into(),
        Type::String => return "`string`".into(),
        Type::ErrorContext => return "`error-context`".into(),
    };
    let def = &resolve.types[id];
    let resource_name =
        |id: wit_parser::TypeId| resolve.types[id].name.clone().unwrap_or_else(|| "_".into());
    match (&def.name, &def.kind) {
        (Some(name), kind) => format!("`{name}` ({})", kind.as_str()),
        (None, TypeDefKind::Handle(Handle::Own(id))) => format!("`own<{}>`", resource_name(*id)),
        (None, TypeDefKind::Handle(Handle::Borrow(id))) => {
            format!("`borrow<{}>`", resource_name(*id))
        }
        (None, kind) => format!("anonymous {}", kind.as_str()),
    }
}
