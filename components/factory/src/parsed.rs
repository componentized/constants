//! Converts WIT parsed by `componentized:component/wit#extract` back into a
//! [`wit_parser::Resolve`].
//!
//! The record mirrors the arenas of the `Resolve` it was extracted from, with
//! ids replaced by strings. Ids are treated as opaque: each is mapped to a
//! newly allocated arena entry, so any consistent naming works.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use wit_parser::{Docs, Resolve, Stability, TypeDef, TypeDefKind, TypeOwner};

use crate::componentized::component::wit as w;

pub use w::Wit;

/// A resolved package, and the world of the component the WIT was extracted
/// from, if any.
pub struct Parsed {
    pub resolve: Resolve,
    pub package: wit_parser::PackageId,
    pub component_world: Option<wit_parser::WorldId>,
}

pub fn to_resolve(wit: w::Wit) -> Result<Parsed> {
    let mut resolve = Resolve {
        all_features: true,
        ..Default::default()
    };
    let mut ids = Ids::default();

    // reserve an arena entry for every item up front, as items reference each
    // other in every direction, then fill them in
    for key in sorted(wit.packages.keys()) {
        let package = &wit.packages[key];
        let id = resolve.packages.alloc(wit_parser::Package {
            name: package_name(&package.name)?,
            docs: Docs::default(),
            interfaces: Default::default(),
            worlds: Default::default(),
        });
        ids.packages.insert(key.clone(), id);
    }
    for key in sorted(wit.interfaces.keys()) {
        let id = resolve.interfaces.alloc(wit_parser::Interface {
            name: None,
            types: Default::default(),
            functions: Default::default(),
            docs: Docs::default(),
            stability: Stability::Unknown,
            package: None,
            span: Default::default(),
            clone_of: None,
        });
        ids.interfaces.insert(key.clone(), id);
    }
    for key in sorted(wit.worlds.keys()) {
        let id = resolve.worlds.alloc(wit_parser::World {
            name: String::new(),
            imports: Default::default(),
            exports: Default::default(),
            package: None,
            docs: Docs::default(),
            stability: Stability::Unknown,
            includes: vec![],
            span: Default::default(),
        });
        ids.worlds.insert(key.clone(), id);
    }
    // types must follow the types they reference
    for key in topological_types(&wit.types)? {
        let id = resolve.types.alloc(TypeDef {
            name: None,
            kind: TypeDefKind::Unknown,
            owner: TypeOwner::None,
            docs: Docs::default(),
            stability: Stability::Unknown,
            span: Default::default(),
            external_id: None,
        });
        ids.types.insert(key, id);
    }

    for (key, type_def) in &wit.types {
        resolve.types[ids.types[key]] = ids.type_def(type_def)?;
    }
    for (key, interface) in &wit.interfaces {
        resolve.interfaces[ids.interfaces[key]] = ids.interface(interface)?;
    }
    for (key, world) in &wit.worlds {
        resolve.worlds[ids.worlds[key]] = ids.world(world)?;
    }
    for (key, package) in &wit.packages {
        let id = ids.packages[key];
        let package = ids.package(package)?;
        resolve.package_names.insert(package.name.clone(), id);
        resolve.packages[id] = package;
    }

    Ok(Parsed {
        package: ids.package_id(&wit.default_package)?,
        component_world: wit
            .component_world
            .as_ref()
            .map(|id| ids.world_id(id))
            .transpose()?,
        resolve,
    })
}

/// Sorts ids naturally, so `type:2` precedes `type:10`. The `extract` ids
/// number items in their original order.
fn sorted<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<&'a String> {
    fn natural(key: &str) -> (&str, Option<u64>, &str) {
        match key.rsplit_once(':') {
            Some((prefix, n)) => (prefix, n.parse().ok(), key),
            None => (key, None, key),
        }
    }
    let mut keys: Vec<&String> = keys.collect();
    keys.sort_by(|a, b| natural(a).cmp(&natural(b)));
    keys
}

/// Orders types so each follows the types it references, which the canonical
/// ABI size and alignment calculations rely on.
fn topological_types(
    types: &std::collections::BTreeMap<String, w::TypeDef>,
) -> Result<Vec<String>> {
    fn referenced(kind: &w::TypeDefKind) -> Vec<&w::Type> {
        match kind {
            w::TypeDefKind::Record(r) => r.fields.iter().map(|f| &f.type_).collect(),
            w::TypeDefKind::Tuple(t) => t.types.iter().collect(),
            w::TypeDefKind::Variant(v) => v.cases.iter().filter_map(|c| c.type_.as_ref()).collect(),
            w::TypeDefKind::Option(t) | w::TypeDefKind::Type(t) => vec![t],
            w::TypeDefKind::Result(r) => r.ok.iter().chain(&r.err).collect(),
            w::TypeDefKind::List(l) => vec![&l.type_],
            w::TypeDefKind::Map(m) => vec![&m.key, &m.value],
            w::TypeDefKind::Future(t) | w::TypeDefKind::Stream(t) => t.iter().collect(),
            w::TypeDefKind::Resource
            | w::TypeDefKind::Handle(_)
            | w::TypeDefKind::Flags(_)
            | w::TypeDefKind::Enum(_)
            | w::TypeDefKind::Unknown => vec![],
        }
    }
    fn dependencies(kind: &w::TypeDefKind) -> Vec<&String> {
        let mut ids: Vec<&String> = referenced(kind)
            .into_iter()
            .filter_map(|t| match t {
                w::Type::Id(id) => Some(id),
                _ => None,
            })
            .collect();
        if let w::TypeDefKind::Handle(w::Handle::Own(id) | w::Handle::Borrow(id)) = kind {
            ids.push(id);
        }
        ids
    }
    fn visit<'a>(
        key: &'a String,
        types: &'a std::collections::BTreeMap<String, w::TypeDef>,
        visiting: &mut HashSet<&'a String>,
        done: &mut HashSet<&'a String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if done.contains(key) {
            return Ok(());
        }
        if !visiting.insert(key) {
            bail!("type `{key}` references itself");
        }
        let type_def = types
            .get(key)
            .ok_or_else(|| anyhow!("unknown type `{key}`"))?;
        for dependency in dependencies(&type_def.kind) {
            visit(dependency, types, visiting, done, order)?;
        }
        visiting.remove(key);
        done.insert(key);
        order.push(key.clone());
        Ok(())
    }

    let (mut visiting, mut done, mut order) = (HashSet::new(), HashSet::new(), vec![]);
    for key in sorted(types.keys()) {
        visit(key, types, &mut visiting, &mut done, &mut order)?;
    }
    Ok(order)
}

#[derive(Default)]
struct Ids {
    packages: HashMap<String, wit_parser::PackageId>,
    interfaces: HashMap<String, wit_parser::InterfaceId>,
    worlds: HashMap<String, wit_parser::WorldId>,
    types: HashMap<String, wit_parser::TypeId>,
}

impl Ids {
    fn package_id(&self, id: &str) -> Result<wit_parser::PackageId> {
        self.packages
            .get(id)
            .copied()
            .ok_or_else(|| anyhow!("unknown package `{id}`"))
    }

    fn interface_id(&self, id: &str) -> Result<wit_parser::InterfaceId> {
        self.interfaces
            .get(id)
            .copied()
            .ok_or_else(|| anyhow!("unknown interface `{id}`"))
    }

    fn world_id(&self, id: &str) -> Result<wit_parser::WorldId> {
        self.worlds
            .get(id)
            .copied()
            .ok_or_else(|| anyhow!("unknown world `{id}`"))
    }

    fn type_id(&self, id: &str) -> Result<wit_parser::TypeId> {
        self.types
            .get(id)
            .copied()
            .ok_or_else(|| anyhow!("unknown type `{id}`"))
    }

    fn package(&self, package: &w::Package) -> Result<wit_parser::Package> {
        Ok(wit_parser::Package {
            name: package_name(&package.name)?,
            docs: docs(&package.docs),
            interfaces: package
                .interfaces
                .iter()
                .map(|(name, id)| Ok((name.clone(), self.interface_id(id)?)))
                .collect::<Result<_>>()?,
            worlds: package
                .worlds
                .iter()
                .map(|(name, id)| Ok((name.clone(), self.world_id(id)?)))
                .collect::<Result<_>>()?,
        })
    }

    fn interface(&self, interface: &w::Interface) -> Result<wit_parser::Interface> {
        Ok(wit_parser::Interface {
            name: interface.name.clone(),
            types: interface
                .types
                .iter()
                .map(|(name, id)| Ok((name.clone(), self.type_id(id)?)))
                .collect::<Result<_>>()?,
            functions: interface
                .functions
                .iter()
                .map(|(name, func)| Ok((name.clone(), self.function(func)?)))
                .collect::<Result<_>>()?,
            docs: docs(&interface.docs),
            stability: stability(&interface.stability)?,
            package: interface
                .package
                .as_deref()
                .map(|id| self.package_id(id))
                .transpose()?,
            span: Default::default(),
            clone_of: None,
        })
    }

    fn world(&self, world: &w::World) -> Result<wit_parser::World> {
        let items = |items: &[(w::WorldKey, w::WorldItem)]| {
            items
                .iter()
                .map(|(key, item)| Ok((self.world_key(key)?, self.world_item(item)?)))
                .collect::<Result<_>>()
        };
        Ok(wit_parser::World {
            name: world.name.clone(),
            imports: items(&world.imports).context("world imports")?,
            exports: items(&world.exports).context("world exports")?,
            package: world
                .package
                .as_deref()
                .map(|id| self.package_id(id))
                .transpose()?,
            docs: docs(&world.docs),
            stability: stability(&world.stability)?,
            includes: world
                .includes
                .iter()
                .map(|include| {
                    Ok(wit_parser::WorldInclude {
                        stability: stability(&include.stability)?,
                        id: self.world_id(&include.id)?,
                        names: include
                            .names
                            .iter()
                            .map(|name| wit_parser::IncludeName {
                                name: name.name.clone(),
                                as_: name.as_.clone(),
                            })
                            .collect(),
                        span: Default::default(),
                    })
                })
                .collect::<Result<_>>()?,
            span: Default::default(),
        })
    }

    fn world_key(&self, key: &w::WorldKey) -> Result<wit_parser::WorldKey> {
        Ok(match key {
            w::WorldKey::Name(name) => wit_parser::WorldKey::Name(name.clone()),
            w::WorldKey::Interface(id) => wit_parser::WorldKey::Interface(self.interface_id(id)?),
        })
    }

    fn world_item(&self, item: &w::WorldItem) -> Result<wit_parser::WorldItem> {
        Ok(match item {
            w::WorldItem::Interface(interface) => wit_parser::WorldItem::Interface {
                id: self.interface_id(&interface.id)?,
                stability: stability(&interface.stability)?,
                external_id: interface.external_id.clone(),
                docs: docs(&interface.docs),
                span: Default::default(),
            },
            w::WorldItem::Function(func) => wit_parser::WorldItem::Function(self.function(func)?),
            w::WorldItem::Type(id) => wit_parser::WorldItem::Type {
                id: self.type_id(id)?,
                span: Default::default(),
            },
        })
    }

    fn function(&self, func: &w::Function) -> Result<wit_parser::Function> {
        use wit_parser::FunctionKind as K;
        Ok(wit_parser::Function {
            name: func.name.clone(),
            kind: match &func.kind {
                w::FunctionKind::Freestanding => K::Freestanding,
                w::FunctionKind::AsyncFreestanding => K::AsyncFreestanding,
                w::FunctionKind::Method(id) => K::Method(self.type_id(id)?),
                w::FunctionKind::AsyncMethod(id) => K::AsyncMethod(self.type_id(id)?),
                w::FunctionKind::Static(id) => K::Static(self.type_id(id)?),
                w::FunctionKind::AsyncStatic(id) => K::AsyncStatic(self.type_id(id)?),
                w::FunctionKind::Constructor(id) => K::Constructor(self.type_id(id)?),
                w::FunctionKind::Getter => K::Getter,
                w::FunctionKind::Setter => K::Setter,
                w::FunctionKind::MethodGetter(id) => K::MethodGetter(self.type_id(id)?),
                w::FunctionKind::MethodSetter(id) => K::MethodSetter(self.type_id(id)?),
                w::FunctionKind::StaticGetter(id) => K::StaticGetter(self.type_id(id)?),
                w::FunctionKind::StaticSetter(id) => K::StaticSetter(self.type_id(id)?),
            },
            params: func
                .params
                .iter()
                .map(|param| {
                    Ok(wit_parser::Param {
                        name: param.name.clone(),
                        ty: self.ty(&param.type_)?,
                        span: Default::default(),
                    })
                })
                .collect::<Result<_>>()?,
            result: func.result.as_ref().map(|t| self.ty(t)).transpose()?,
            docs: docs(&func.docs),
            stability: stability(&func.stability)?,
            span: Default::default(),
            external_id: func.external_id.clone(),
        })
    }

    fn ty(&self, ty: &w::Type) -> Result<wit_parser::Type> {
        use wit_parser::Type as T;
        Ok(match ty {
            w::Type::Bool => T::Bool,
            w::Type::S8 => T::S8,
            w::Type::S16 => T::S16,
            w::Type::S32 => T::S32,
            w::Type::S64 => T::S64,
            w::Type::U8 => T::U8,
            w::Type::U16 => T::U16,
            w::Type::U32 => T::U32,
            w::Type::U64 => T::U64,
            w::Type::F32 => T::F32,
            w::Type::F64 => T::F64,
            w::Type::Char => T::Char,
            w::Type::String => T::String,
            w::Type::ErrorContext => T::ErrorContext,
            w::Type::Id(id) => T::Id(self.type_id(id)?),
        })
    }

    fn opt_ty(&self, ty: &Option<w::Type>) -> Result<Option<wit_parser::Type>> {
        ty.as_ref().map(|t| self.ty(t)).transpose()
    }

    fn type_def(&self, type_def: &w::TypeDef) -> Result<TypeDef> {
        use TypeDefKind as K;
        let kind = match &type_def.kind {
            w::TypeDefKind::Record(r) => K::Record(wit_parser::Record {
                fields: r
                    .fields
                    .iter()
                    .map(|f| {
                        Ok(wit_parser::Field {
                            name: f.name.clone(),
                            ty: self.ty(&f.type_)?,
                            docs: docs(&f.docs),
                            span: Default::default(),
                        })
                    })
                    .collect::<Result<_>>()?,
            }),
            w::TypeDefKind::Resource => K::Resource,
            w::TypeDefKind::Handle(w::Handle::Own(id)) => {
                K::Handle(wit_parser::Handle::Own(self.type_id(id)?))
            }
            w::TypeDefKind::Handle(w::Handle::Borrow(id)) => {
                K::Handle(wit_parser::Handle::Borrow(self.type_id(id)?))
            }
            w::TypeDefKind::Flags(f) => K::Flags(wit_parser::Flags {
                flags: f
                    .flags
                    .iter()
                    .map(|f| wit_parser::Flag {
                        name: f.name.clone(),
                        docs: docs(&f.docs),
                        span: Default::default(),
                    })
                    .collect(),
            }),
            w::TypeDefKind::Tuple(t) => K::Tuple(wit_parser::Tuple {
                types: t.types.iter().map(|t| self.ty(t)).collect::<Result<_>>()?,
            }),
            w::TypeDefKind::Variant(v) => K::Variant(wit_parser::Variant {
                cases: v
                    .cases
                    .iter()
                    .map(|c| {
                        Ok(wit_parser::Case {
                            name: c.name.clone(),
                            ty: self.opt_ty(&c.type_)?,
                            docs: docs(&c.docs),
                            span: Default::default(),
                        })
                    })
                    .collect::<Result<_>>()?,
            }),
            w::TypeDefKind::Enum(e) => K::Enum(wit_parser::Enum {
                cases: e
                    .cases
                    .iter()
                    .map(|c| wit_parser::EnumCase {
                        name: c.name.clone(),
                        docs: docs(&c.docs),
                        span: Default::default(),
                    })
                    .collect(),
            }),
            w::TypeDefKind::Option(t) => K::Option(self.ty(t)?),
            w::TypeDefKind::Result(r) => K::Result(wit_parser::Result_ {
                ok: self.opt_ty(&r.ok)?,
                err: self.opt_ty(&r.err)?,
            }),
            w::TypeDefKind::List(l) => match l.fixed_length {
                Some(n) => K::FixedLengthList(self.ty(&l.type_)?, n),
                None => K::List(self.ty(&l.type_)?),
            },
            w::TypeDefKind::Map(m) => K::Map(self.ty(&m.key)?, self.ty(&m.value)?),
            w::TypeDefKind::Future(t) => K::Future(self.opt_ty(t)?),
            w::TypeDefKind::Stream(t) => K::Stream(self.opt_ty(t)?),
            w::TypeDefKind::Type(t) => K::Type(self.ty(t)?),
            w::TypeDefKind::Unknown => K::Unknown,
        };
        Ok(TypeDef {
            name: type_def.name.clone(),
            kind,
            owner: match &type_def.owner {
                w::TypeOwner::World(id) => TypeOwner::World(self.world_id(id)?),
                w::TypeOwner::Interface(id) => TypeOwner::Interface(self.interface_id(id)?),
                w::TypeOwner::None => TypeOwner::None,
            },
            docs: docs(&type_def.docs),
            stability: stability(&type_def.stability)?,
            span: Default::default(),
            external_id: type_def.external_id.clone(),
        })
    }
}

fn docs(docs: &w::Docs) -> Docs {
    Docs {
        contents: docs.contents.clone(),
    }
}

fn stability(stability: &w::Stability) -> Result<Stability> {
    Ok(match stability {
        w::Stability::Unknown => Stability::Unknown,
        w::Stability::Unstable(u) => Stability::Unstable {
            feature: u.feature.clone(),
            deprecated: u.deprecated.as_ref().map(version).transpose()?,
        },
        w::Stability::Stable(s) => Stability::Stable {
            since: version(&s.since)?,
            deprecated: s.deprecated.as_ref().map(version).transpose()?,
        },
    })
}

fn package_name(name: &w::PackageName) -> Result<wit_parser::PackageName> {
    Ok(wit_parser::PackageName {
        namespace: name.namespace.clone(),
        name: name.name.clone(),
        version: name.version.as_ref().map(version).transpose()?,
    })
}

fn version(version: &w::Version) -> Result<semver::Version> {
    let identifiers = |ids: &Option<Vec<w::VersionIdentifier>>| {
        ids.iter()
            .flatten()
            .map(|id| match id {
                w::VersionIdentifier::String(s) => s.clone(),
                w::VersionIdentifier::Numeric(n) => n.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".")
    };
    let mut text = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let pre = identifiers(&version.prerelease);
    if !pre.is_empty() {
        text = format!("{text}-{pre}");
    }
    let build = identifiers(&version.build_metadata);
    if !build.is_empty() {
        text = format!("{text}+{build}");
    }
    semver::Version::parse(&text).with_context(|| format!("invalid version `{text}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn docs(contents: Option<&str>) -> w::Docs {
        w::Docs {
            contents: contents.map(String::from),
        }
    }

    /// A `list<t1>` alias whose id sorts before the `u8` alias it references,
    /// using ids that don't follow the `extract` naming.
    fn wit() -> w::Wit {
        let type_def = |name: &str, kind| w::TypeDef {
            name: Some(name.into()),
            kind,
            owner: w::TypeOwner::Interface("the-interface".into()),
            docs: docs(None),
            stability: w::Stability::Unknown,
            external_id: None,
        };
        let types = BTreeMap::from([
            (
                "a".to_string(),
                type_def(
                    "bytes",
                    w::TypeDefKind::List(w::List {
                        type_: w::Type::Id("z".into()),
                        fixed_length: None,
                    }),
                ),
            ),
            (
                "z".to_string(),
                type_def("byte", w::TypeDefKind::Type(w::Type::U8)),
            ),
        ]);
        let function = w::Function {
            name: "data".into(),
            kind: w::FunctionKind::Freestanding,
            params: vec![],
            result: Some(w::Type::Id("a".into())),
            docs: docs(Some("@value [1, 2, 3]")),
            stability: w::Stability::Unknown,
            external_id: None,
        };
        let interface = w::Interface {
            name: Some("constants".into()),
            types: vec![("bytes".into(), "a".into()), ("byte".into(), "z".into())],
            functions: vec![("data".into(), function)],
            docs: docs(None),
            stability: w::Stability::Unknown,
            package: Some("the-package".into()),
        };
        let world = w::World {
            name: "config".into(),
            imports: vec![],
            exports: vec![(
                w::WorldKey::Interface("the-interface".into()),
                w::WorldItem::Interface(w::WorldItemInterface {
                    id: "the-interface".into(),
                    stability: w::Stability::Unknown,
                    external_id: None,
                    docs: docs(None),
                }),
            )],
            package: Some("the-package".into()),
            docs: docs(None),
            stability: w::Stability::Unknown,
            includes: vec![],
        };
        let package = w::Package {
            name: w::PackageName {
                namespace: "example".into(),
                name: "parsed".into(),
                version: Some(w::Version {
                    major: 1,
                    minor: 2,
                    patch: 3,
                    prerelease: Some(vec![
                        w::VersionIdentifier::String("rc".into()),
                        w::VersionIdentifier::Numeric(1),
                    ]),
                    build_metadata: None,
                }),
            },
            docs: docs(None),
            interfaces: vec![("constants".into(), "the-interface".into())],
            worlds: vec![("config".into(), "the-world".into())],
        };
        w::Wit {
            interfaces: BTreeMap::from([("the-interface".to_string(), interface)]),
            packages: BTreeMap::from([("the-package".to_string(), package)]),
            types,
            worlds: BTreeMap::from([("the-world".to_string(), world)]),
            default_package: "the-package".into(),
            component_world: None,
        }
    }

    #[test]
    fn it_orders_types_by_dependency() -> Result<()> {
        let parsed = to_resolve(wit())?;
        let resolve = &parsed.resolve;
        let names: Vec<_> = resolve
            .types
            .iter()
            .map(|(_, t)| t.name.clone().unwrap())
            .collect();
        assert_eq!(names, ["byte", "bytes"]);
        assert_eq!(
            resolve.id_of(resolve.packages[parsed.package].interfaces["constants"]),
            Some("example:parsed/constants@1.2.3-rc.1".into())
        );

        let world = resolve.select_world(&[parsed.package], None)?;
        componentized_constants::create_component(resolve, world, None)?;
        Ok(())
    }

    #[test]
    fn it_rejects_unknown_ids() {
        let mut wit = wit();
        wit.types.remove("z");
        let err = to_resolve(wit).err().expect("unknown type");
        assert_eq!(err.to_string(), "unknown type `z`");
    }
}
