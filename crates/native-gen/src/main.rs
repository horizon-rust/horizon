//! Generator: reads `horizon.wit` and writes plain Rust —
//! types, `Host` traits (host side), guest functions that dispatch through
//! a thread-local host, `Guest` traits (the game's calls into a mod) and a
//! native vtable plus per-world handle types that mirror the shape of
//! wasmtime's `bindgen!` output closely enough that `runtime.rs` keeps
//! compiling with only wiring changes. No wasmtime anywhere.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use wit_parser::{Resolve, Type, TypeDefKind, TypeId, WorldItem, WorldKey};

/// The worlds whose exports the game calls (one per capability world, plus
/// the two base ones); a handle type is generated for each.
const HOST_WORLDS: &[&str] = &[
    "horizon-mod",
    "horizon-server-mod",
    "ui-host",
    "gui-screens-host",
    "block-entity-host",
    "menu-host",
    "use-host",
    "multiblock-host",
    "item-host",
    "net-server-host",
    "net-client-host",
    "entity-host",
    "redstone-host",
    "game-events-host",
    "rich-events-host",
    "config-host",
    "inter-mod-host",
];
/// Worlds whose exports are `Guest` traits (the base worlds' events are
/// exported by both guest worlds, so collect from them too).
const EVENT_SOURCE_WORLDS: &[&str] = &[
    "horizon-ui-mod",
    "horizon-block-mod",
    "horizon-mod",
    "horizon-server-mod",
];

const RESERVED: &[&str] = &[
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "async", "await", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield",
];

fn snake(name: &str) -> String {
    let mut s = name.replace(['-', '@'], "_");
    s.retain(|c| c != '%');
    if RESERVED.contains(&s.as_str()) {
        s.push('_');
    }
    s
}

fn camel(name: &str) -> String {
    name.split(['-', '@', '%'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut cs = p.chars();
            match cs.next() {
                Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn main() {
    let mut args = std::env::args_os();
    let _ = args.next();
    let wit = args
        .next()
        .expect("usage: native-gen <horizon.wit> <out.rs>");
    let out = args
        .next()
        .expect("usage: native-gen <horizon.wit> <out.rs>");
    let text = std::fs::read_to_string(&wit).expect("read the wit file");
    let mut resolve = Resolve::new();
    let group = wit_parser::UnresolvedPackageGroup::parse(&wit, &text)
        .map_err(|(_, e)| e.to_string())
        .expect("parse the wit file");
    let package = resolve.push(group.main, 0).expect("resolve the wit file");

    // Every interface of the package, by name.
    let mut ifaces: BTreeMap<String, wit_parser::InterfaceId> = BTreeMap::new();
    for (id, i) in resolve.interfaces.iter() {
        if i.package == Some(package)
            && let Some(name) = &i.name
        {
            ifaces.insert(name.clone(), id);
        }
    }
    // The events interfaces: exported by the guest or base worlds.
    let mut event_ifaces: BTreeMap<String, wit_parser::InterfaceId> = BTreeMap::new();
    for (_id, w) in resolve.worlds.iter() {
        if w.package == Some(package) && EVENT_SOURCE_WORLDS.contains(&w.name.as_str()) {
            for (_, item) in w.exports.iter() {
                if let WorldItem::Interface { id, .. } = item {
                    let name = resolve.interfaces[*id]
                        .name
                        .clone()
                        .expect("named interface");
                    event_ifaces.insert(name, *id);
                }
            }
        }
    }
    let host_ifaces: BTreeMap<String, wit_parser::InterfaceId> = ifaces
        .iter()
        .filter(|(n, _)| !event_ifaces.contains_key(n.as_str()))
        .map(|(n, id)| (n.clone(), *id))
        .collect();
    eprintln!(
        "interfaces: {} total, {} events, {} host",
        ifaces.len(),
        event_ifaces.len(),
        host_ifaces.len()
    );

    let g = Gen { resolve: &resolve };
    let mut code = String::new();
    code.push_str(GENERATED_HEADER);
    // One `horizon::mods` tree with every interface in it.
    code.push_str("pub mod horizon { pub mod mods {\n");
    for (name, id) in &host_ifaces {
        g.iface(&mut code, name, *id);
    }
    // The events interfaces' types live beside the others.
    for (name, id) in &event_ifaces {
        g.iface_types(&mut code, name, *id);
    }
    code.push_str("} }\n\npub mod exports { pub mod horizon { pub mod mods {\n");
    for (name, id) in &event_ifaces {
        g.guest_trait(&mut code, name, *id);
    }
    code.push_str("} } }\n");
    // HostAll, the thread-local dispatch, NativeStore.
    code.push_str(&g.host_all(&host_ifaces));
    // The vtable the SDK macro fills and the handles the game calls.
    code.push_str(&g.vtable(&event_ifaces));
    for world in HOST_WORLDS {
        g.handle(&mut code, world, &resolve, package, &event_ifaces);
    }
    // The file is committed, so it ships in the one shape rustfmt leaves
    // it in; without rustfmt (never, in a rustup toolchain) write it raw.
    let code =
        std::process::Command::new(std::env::var_os("RUSTFMT").unwrap_or_else(|| "rustfmt".into()))
            .arg("--edition")
            .arg("2024")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child.stdin.as_mut().unwrap().write_all(code.as_bytes())?;
                child.wait_with_output()
            })
            .map(|out| String::from_utf8(out.stdout).expect("rustfmt writes UTF-8"))
            .unwrap_or_else(|e| {
                eprintln!("native-gen: rustfmt not run ({e}); the file is written unformatted");
                code
            });
    std::fs::write(&out, code).expect("write the generated file");
}

const GENERATED_HEADER: &str = r#"//! GENERATED by `native-gen` from `wit/horizon.wit` — do not
//! edit: plain Rust types, Host traits, guest functions, Guest traits and a
//! native vtable replacing wasmtime's `bindgen!`.
#![allow(dead_code, unused_imports, unused_variables, unused_mut, clippy::all)]

use std::cell::RefCell;

pub type NativeError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The host side of every interface, as one trait: the game's `State`
/// implements each `Host` trait, so it is an `HostAll` for free.
"#;

struct Gen<'a> {
    resolve: &'a Resolve,
}

impl Gen<'_> {
    /// A host interface: its types, its `Host` trait and its guest
    /// functions, in `native::horizon::mods::<name>`.
    fn iface(&self, code: &mut String, name: &str, id: wit_parser::InterfaceId) {
        let m = snake(name);
        let _ = writeln!(code, "\npub mod {m} {{");
        self.types(code, id);
        self.host_trait(code, name, id);
        self.guest_fns(code, name, id);
        code.push_str("}\n");
    }

    /// An events interface's types only (the `Guest` trait is separate).
    fn iface_types(&self, code: &mut String, name: &str, id: wit_parser::InterfaceId) {
        let m = snake(name);
        let _ = writeln!(code, "\npub mod {m} {{");
        self.types(code, id);
        code.push_str("}\n");
    }

    fn types(&self, code: &mut String, id: wit_parser::InterfaceId) {
        let iface = &self.resolve.interfaces[id];
        for (name, ty) in iface.types.iter() {
            self.type_def(code, name, *ty);
        }
    }

    /// A record's derives: `Copy` beside the rest when it holds plain data
    /// only, as wit-bindgen had it (the SDK relies on that, e.g.
    /// `ArgumentValue::BlockPos(p) => Some(*p)`).
    fn record_derives(&self, owns_nothing: bool) -> &'static str {
        if owns_nothing {
            "#[derive(Clone, Copy, Debug, PartialEq)]"
        } else {
            "#[derive(Clone, Debug, PartialEq)]"
        }
    }

    /// A variant's derives: as a record's, minus `Debug` — wit-bindgen's
    /// `Debug` prints the variant qualified (`Kind::Brewing`), and a
    /// derived one would not, so [`Self::enum_debug`] writes it by hand.
    fn variant_derives(&self, owns_nothing: bool) -> &'static str {
        if owns_nothing {
            "#[derive(Clone, Copy, PartialEq)]"
        } else {
            "#[derive(Clone, PartialEq)]"
        }
    }

    /// `Debug` as wit-bindgen wrote it for an enum or a variant: the case
    /// qualified by its type's name (`Kind::Brewing`, `Hit::Block(e)`), so
    /// a mod's `{:?}` prints the same on both paths.
    fn enum_debug(&self, code: &mut String, rust: &str, cases: &[(String, bool)]) {
        let _ = writeln!(
            code,
            "impl std::fmt::Debug for {rust} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self {{"
        );
        for (case, payload) in cases {
            let _ = writeln!(
                code,
                "            {rust}::{case}{bind} => f.debug_tuple(\"{rust}::{case}\"){field}.finish(),",
                bind = if *payload { "(e)" } else { "" },
                field = if *payload { ".field(e)" } else { "" },
            );
        }
        code.push_str("        }\n    }\n}\n");
    }

    fn type_def(&self, code: &mut String, name: &str, ty: TypeId) {
        let def = &self.resolve.types[ty];
        let rust = camel(name);
        match &def.kind {
            TypeDefKind::Record(r) => {
                let _ = writeln!(
                    code,
                    "{}\npub struct {rust} {{",
                    self.record_derives(r.fields.iter().all(|f| self.owns_nothing(&f.ty)))
                );
                for f in &r.fields {
                    let _ = writeln!(code, "    pub {}: {},", snake(&f.name), self.ty(&f.ty));
                }
                code.push_str("}\n");
            }
            TypeDefKind::Variant(v) => {
                let owns = v
                    .cases
                    .iter()
                    .all(|c| c.ty.as_ref().is_none_or(|t| self.owns_nothing(t)));
                let _ = writeln!(code, "{}\npub enum {rust} {{", self.variant_derives(owns));
                let mut cases = Vec::new();
                for c in &v.cases {
                    match &c.ty {
                        Some(t) => {
                            let _ = writeln!(code, "    {}({}),", camel(&c.name), self.ty(t));
                            cases.push((camel(&c.name), true));
                        }
                        None => {
                            let _ = writeln!(code, "    {},", camel(&c.name));
                            cases.push((camel(&c.name), false));
                        }
                    }
                }
                code.push_str("}\n");
                self.enum_debug(code, &rust, &cases);
            }
            TypeDefKind::Enum(e) => {
                let _ = writeln!(
                    code,
                    "#[derive(Clone, Copy, PartialEq, Eq, Hash)]\npub enum {rust} {{"
                );
                for c in &e.cases {
                    let _ = writeln!(code, "    {},", camel(&c.name));
                }
                code.push_str("}\n");
                let cases: Vec<(String, bool)> =
                    e.cases.iter().map(|c| (camel(&c.name), false)).collect();
                self.enum_debug(code, &rust, &cases);
            }
            TypeDefKind::Tuple(t) => {
                let parts: Vec<String> = t.types.iter().map(|t| self.ty(t)).collect();
                let _ = writeln!(code, "pub type {rust} = ({});", parts.join(", "));
            }
            TypeDefKind::List(inner) => {
                let _ = writeln!(code, "pub type {rust} = Vec<{}>;", self.ty(inner));
            }
            TypeDefKind::Option(inner) => {
                let _ = writeln!(code, "pub type {rust} = Option<{}>;", self.ty(inner));
            }
            TypeDefKind::Result(r) => {
                let ok = r.ok.as_ref().map_or("()".into(), |t| self.ty(t));
                let err = r.err.as_ref().map_or("()".into(), |t| self.ty(t));
                let _ = writeln!(code, "pub type {rust} = Result<{ok}, {err}>;");
            }
            TypeDefKind::Type(t) => {
                let _ = writeln!(code, "pub type {rust} = {};", self.ty(t));
            }
            other => panic!("type {name}: unsupported kind {other:?}"),
        }
    }

    /// A type as the host and the guest both see it, by value.
    fn ty(&self, t: &Type) -> String {
        match t {
            Type::Bool => "bool".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::S8 => "i8".into(),
            Type::S16 => "i16".into(),
            Type::S32 => "i32".into(),
            Type::S64 => "i64".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Char => "char".into(),
            Type::String => "String".into(),
            Type::ErrorContext => panic!("error-context is not supported"),
            Type::Id(id) => {
                let def = &self.resolve.types[*id];
                if let Some(name) = &def.name {
                    let owner = match def.owner {
                        wit_parser::TypeOwner::Interface(i) => i,
                        wit_parser::TypeOwner::World(_) => {
                            panic!("type {name} owned by a world")
                        }
                        wit_parser::TypeOwner::None => panic!("type {name} has no owner"),
                    };
                    let iface = self.resolve.interfaces[owner]
                        .name
                        .clone()
                        .expect("named interface");
                    return format!(
                        "crate::native::horizon::mods::{}::{}",
                        snake(&iface),
                        camel(name)
                    );
                }
                match &def.kind {
                    TypeDefKind::List(inner) => format!("Vec<{}>", self.ty(inner)),
                    TypeDefKind::Option(inner) => format!("Option<{}>", self.ty(inner)),
                    TypeDefKind::Tuple(t) => {
                        let parts: Vec<String> = t.types.iter().map(|t| self.ty(t)).collect();
                        format!("({})", parts.join(", "))
                    }
                    TypeDefKind::Result(r) => {
                        let ok = r.ok.as_ref().map_or("()".into(), |t| self.ty(t));
                        let err = r.err.as_ref().map_or("()".into(), |t| self.ty(t));
                        format!("Result<{ok}, {err}>")
                    }
                    TypeDefKind::Type(t) => self.ty(t),
                    other => panic!("anonymous type: {other:?}"),
                }
            }
        }
    }

    /// Whether a type owns heap data (strings, lists, owning records): such
    /// values cross the call boundary by reference, the rest by value — the
    /// same shape wasmtime's bindgen gives its `call_*` methods.
    fn owns_nothing(&self, t: &Type) -> bool {
        match t {
            Type::Bool
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::S8
            | Type::S16
            | Type::S32
            | Type::S64
            | Type::F32
            | Type::F64
            | Type::Char => true,
            Type::String | Type::ErrorContext => false,
            Type::Id(id) => match &self.resolve.types[*id].kind {
                TypeDefKind::Record(r) => r.fields.iter().all(|f| self.owns_nothing(&f.ty)),
                TypeDefKind::Variant(v) => v
                    .cases
                    .iter()
                    .all(|c| c.ty.as_ref().is_none_or(|t| self.owns_nothing(t))),
                TypeDefKind::Enum(_) => true,
                TypeDefKind::Tuple(t) => t.types.iter().all(|t| self.owns_nothing(t)),
                TypeDefKind::Option(inner) => self.owns_nothing(inner),
                TypeDefKind::Type(t) => self.owns_nothing(t),
                _ => false,
            },
        }
    }

    /// The interface's `Host` trait: methods take and return owned values,
    /// exactly as the 30 existing `impl … Host for State` blocks expect.
    fn host_trait(&self, code: &mut String, _name: &str, id: wit_parser::InterfaceId) {
        let iface = &self.resolve.interfaces[id];
        code.push_str("\npub trait Host {\n");
        for f in iface.functions.values() {
            let params = self.params_owning(f);
            let ret = self.ret_owning(f);
            let _ = writeln!(
                code,
                "    fn {}({}){};",
                snake(&f.name),
                params,
                ret.as_ref().map_or(String::new(), |r| format!(" -> {r}"))
            );
        }
        code.push_str("}\n");
    }

    /// The guest functions a mod calls: borrowed parameters, owned results,
    /// dispatching to the thread-local host.
    fn guest_fns(&self, code: &mut String, _name: &str, id: wit_parser::InterfaceId) {
        let iface = &self.resolve.interfaces[id];
        for f in iface.functions.values() {
            let mut params = String::new();
            let mut args = String::new();
            for p in &f.params {
                let n = snake(&p.name);
                let (decl, arg) = self.param_borrowed(&n, &p.ty);
                let _ = write!(params, "{n}: {decl}, ");
                let _ = write!(args, "{arg}, ");
            }
            let ret = self.ret_owning(f);
            let _ = writeln!(
                code,
                "\npub fn {}({}){} {{\n    crate::native::with_host(|h| Host::{}(h, {}))\n}}",
                snake(&f.name),
                params.trim_end_matches(", "),
                ret.as_ref().map_or(String::new(), |r| format!(" -> {r}")),
                snake(&f.name),
                args.trim_end_matches(", "),
            );
        }
    }

    fn params_owning(&self, f: &wit_parser::Function) -> String {
        format!("&mut self, {}", self.params_list(f))
    }

    /// The function's parameters, owned, without `self`.
    fn params_list(&self, f: &wit_parser::Function) -> String {
        let mut parts = Vec::new();
        for p in &f.params {
            parts.push(format!("{}: {}", snake(&p.name), self.ty(&p.ty)));
        }
        parts.join(", ")
    }

    fn ret_owning(&self, f: &wit_parser::Function) -> Option<String> {
        f.result.as_ref().map(|r| self.ty(r))
    }

    /// A borrowed parameter for a guest function, and the expression that
    /// makes the owned value the Host method takes.
    fn param_borrowed(&self, name: &str, t: &Type) -> (String, String) {
        match t {
            Type::String => ("&str".into(), format!("{name}.to_string()")),
            Type::Id(id) => {
                let def = &self.resolve.types[*id];
                match &def.kind {
                    TypeDefKind::List(inner) => {
                        (format!("&[{}]", self.ty(inner)), format!("{name}.to_vec()"))
                    }
                    TypeDefKind::Option(inner) => {
                        if self.owns_nothing(inner) {
                            (self.ty(t), name.to_string())
                        } else {
                            (
                                format!("Option<&{}>", self.ty(inner)),
                                format!("{name}.cloned()"),
                            )
                        }
                    }
                    TypeDefKind::Type(inner) => self.param_borrowed(name, inner),
                    _ => {
                        // A record or variant: by reference if it owns data,
                        // by value if it is plain data.
                        if self.owns_nothing(t) {
                            (self.ty(t), name.to_string())
                        } else {
                            (format!("&{}", self.ty(t)), format!("{name}.clone()"))
                        }
                    }
                }
            }
            _ => (self.ty(t), name.to_string()),
        }
    }

    /// A `call_*` parameter: same borrowing as the guest function's for
    /// strings and lists; plain-data records cross by value (the shape
    /// `runtime.rs` already uses, e.g. `redstone-events.input-changed`).
    fn param_call(&self, name: &str, t: &Type) -> (String, String) {
        match t {
            Type::String => ("&str".into(), format!("{name}.to_string()")),
            Type::Id(id) => match &self.resolve.types[*id].kind {
                TypeDefKind::List(inner) => {
                    (format!("&[{}]", self.ty(inner)), format!("{name}.to_vec()"))
                }
                TypeDefKind::Option(inner) => {
                    if self.owns_nothing(inner) {
                        (self.ty(t), name.to_string())
                    } else {
                        (
                            format!("Option<&{}>", self.ty(inner)),
                            format!("{name}.cloned()"),
                        )
                    }
                }
                TypeDefKind::Type(inner) => self.param_call(name, inner),
                _ => {
                    if self.owns_nothing(t) {
                        (self.ty(t), name.to_string())
                    } else {
                        (format!("&{}", self.ty(t)), format!("{name}.clone()"))
                    }
                }
            },
            _ => (self.ty(t), name.to_string()),
        }
    }

    /// The `Guest` trait of an events interface, under `exports` where
    /// wit-bindgen put it (the SDK's `pub use` lines keep working).
    fn guest_trait(&self, code: &mut String, name: &str, id: wit_parser::InterfaceId) {
        let m = snake(name);
        let iface = &self.resolve.interfaces[id];
        let _ = writeln!(
            code,
            "\npub mod {m} {{\n\
             pub use crate::native::horizon::mods::{m}::*;\n\
             pub trait Guest {{"
        );
        for f in iface.functions.values() {
            let params = self.params_list(f);
            let ret = self.ret_owning(f);
            let _ = writeln!(
                code,
                "    fn {}({}){};",
                snake(&f.name),
                params,
                ret.as_ref().map_or(String::new(), |r| format!(" -> {r}"))
            );
        }
        code.push_str("}\n}\n");
    }

    fn host_all(&self, host_ifaces: &BTreeMap<String, wit_parser::InterfaceId>) -> String {
        let mut code = String::from("\npub trait HostAll:");
        for name in host_ifaces.keys() {
            let _ = write!(code, " horizon::mods::{}::Host +", snake(name));
        }
        // The trailing ' +'.
        code.truncate(code.len() - 1);
        code.push_str(" {}\nimpl<T");
        for (i, name) in host_ifaces.keys().enumerate() {
            let sep = if i == 0 { ":" } else { " +" };
            let _ = write!(code, "{sep} horizon::mods::{}::Host", snake(name));
        }
        code.push_str("> HostAll for T {}\n");
        code.push_str(
            r#"
thread_local! {
    /// The host the guest's calls reach, while a `call_*` runs: one mod,
    /// one thread, one call at a time.
    static HOST: RefCell<Option<*mut dyn HostAll>> = const { RefCell::new(None) };
}
pub(crate) fn with_host<R>(f: impl FnOnce(&mut dyn HostAll) -> R) -> R {
    let host = HOST.with(|s| s.borrow().expect("the host is not running"));
    f(unsafe { &mut *host })
}

/// The game's state for one running mod, in place of wasmtime's `Store`.
pub struct NativeStore<S> {
    state: S,
}
impl<S> NativeStore<S> {
    pub fn new(state: S) -> Self {
        Self { state }
    }
    pub fn data(&self) -> &S {
        &self.state
    }
    pub fn data_mut(&mut self) -> &mut S {
        &mut self.state
    }
}

/// One call into the mod: the host is in place for what the mod calls back,
/// and a panic comes back as an error, as a trap did. The host that was in
/// place before (an outer call's, if a host hook reached mod code again)
/// is restored afterwards, so a nested call cannot leave another mod's
/// host behind.
pub fn call_guest<S, R>(store: &mut NativeStore<S>, f: impl FnOnce() -> R) -> Result<R, NativeError>
where
    S: HostAll + 'static,
{
    let host: *mut dyn HostAll = store.data_mut();
    let outer = HOST.with(|s| s.borrow_mut().replace(host));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    HOST.with(|s| *s.borrow_mut() = outer);
    caught.map_err(|panic| -> NativeError {
        if let Some(text) = panic.downcast_ref::<String>() {
            format!("the mod panicked: {text}").into()
        } else if let Some(text) = panic.downcast_ref::<&str>() {
            format!("the mod panicked: {text}").into()
        } else {
            "the mod panicked".into()
        }
    })
}
"#,
        );
        code
    }

    /// The vtable the SDK's export macro fills in: one section per events
    /// interface, `None` for the ones the mod does not export.
    fn vtable(&self, event_ifaces: &BTreeMap<String, wit_parser::InterfaceId>) -> String {
        let mut code = String::from(
            "\n/// The mod's exports, as function pointers: the SDK's export macro fills\n/// it in for a mod compiled into the process.\n#[derive(Clone, Copy)]\npub struct NativeVtable {\n",
        );
        for name in event_ifaces.keys() {
            let _ = writeln!(code, "    pub {}: Option<{}Vt>,", snake(name), camel(name));
        }
        code.push_str("}\n");
        for (name, id) in event_ifaces {
            let iface = &self.resolve.interfaces[*id];
            let _ = writeln!(
                code,
                "\n#[derive(Clone, Copy)]\npub struct {}Vt {{",
                camel(name)
            );
            for f in iface.functions.values() {
                let params = self.params_list(f);
                let ret = self.ret_owning(f);
                let _ = writeln!(
                    code,
                    "    pub {}: fn({}){},",
                    snake(&f.name),
                    params,
                    ret.as_ref().map_or(String::new(), |r| format!(" -> {r}"))
                );
            }
            code.push_str("}\n");
        }
        code
    }

    /// A handle for a host world: `new` says whether the mod exports the
    /// world's interface, `call_*` methods call into it. Worlds with one
    /// export also answer `horizon_mods_<name>()` with themselves, so the
    /// call sites keep wasmtime bindgen's shape.
    fn handle(
        &self,
        code: &mut String,
        world: &str,
        resolve: &Resolve,
        package: wit_parser::PackageId,
        _event_ifaces: &BTreeMap<String, wit_parser::InterfaceId>,
    ) {
        let wid = resolve
            .worlds
            .iter()
            .find(|(_, w)| w.package == Some(package) && w.name == world)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("world {world} not found"));
        let world_def = &resolve.worlds[wid];
        let mut exports: Vec<(String, wit_parser::InterfaceId)> = Vec::new();
        for (key, item) in world_def.exports.iter() {
            if let WorldItem::Interface { id, .. } = item {
                let name = match key {
                    WorldKey::Name(n) => n.clone(),
                    WorldKey::Interface(_) => resolve.interfaces[*id]
                        .name
                        .clone()
                        .expect("named interface"),
                };
                exports.push((name, *id));
            }
        }
        let handle_name = camel(world);
        let (main, main_id) = exports
            .first()
            .map(|(n, id)| (n.clone(), *id))
            .unwrap_or_else(|| panic!("world {world} has no interface export"));
        let vt_field = snake(&main);
        let vt_type = camel(&main);
        let _ = writeln!(
            code,
            "\n/// The game's handle to a running mod's `{main}` export (world `{world}`).\n\
             #[derive(Clone, Copy)]\npub struct {handle_name} {{ vt: {vt_type}Vt }}\n\
             impl {handle_name} {{\n\
             \x20   pub fn new(vt: &NativeVtable) -> Option<Self> {{\n\
             \x20       Some(Self {{ vt: vt.{vt_field}? }})\n\
             \x20   }}\n\
             \x20   pub fn horizon_mods_{vt_field}(&self) -> &Self {{\n\
             \x20       self\n\
             \x20   }}"
        );
        let iface = &resolve.interfaces[main_id];
        for f in iface.functions.values() {
            let fname = snake(&f.name);
            let mut params = String::new();
            let mut args = String::new();
            for p in &f.params {
                let n = snake(&p.name);
                let (decl, arg) = self.param_call(&n, &p.ty);
                let _ = write!(params, "{n}: {decl}, ");
                let _ = write!(args, "{arg}, ");
            }
            // The guest function's own return type (a WIT `result` is a
            // `Result<_, String>` in it) wrapped once more for the
            // panic/trap layer, as wasmtime's `call_*` methods are.
            let inner = self.ret_owning(f).unwrap_or_else(|| "()".into());
            let call_ret = format!("Result<{inner}, NativeError>");
            let _ = writeln!(
                code,
                "    pub fn call_{fname}<S: HostAll + 'static>(&self, store: &mut NativeStore<S>, {}) -> {} {{\n\
                 \x20       call_guest(store, || (self.vt.{fname})({}))\n\
                 \x20   }}",
                params.trim_end_matches(", "),
                call_ret,
                args.trim_end_matches(", "),
            );
        }
        code.push_str("}\n");
    }
}
