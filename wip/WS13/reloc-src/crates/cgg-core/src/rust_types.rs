//! Rust type-shape rules shared by the Rust plugin, type propagation and
//! the cross-file resolver, so all three agree on what a hint means.

/// Generic types whose methods are never their argument's: std containers
/// and smart pointers (handled by their own rules), channels, handles and
/// futures. A hint `Outer<Inner>` with any other external `Outer` — axum's
/// `State<AppState>`, actix's `web::Data<AppState>`, `FederationConfig<Ctx>`
/// — is treated as `Deref`-ing to `Inner` when `Outer` itself has no such
/// method.
const NOT_DEREF_TO_ARG: &[&str] = &[
    "Vec",
    "VecDeque",
    "LinkedList",
    "Option",
    "Result",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "BTreeSet",
    "BinaryHeap",
    "IndexMap",
    "IndexSet",
    "Box",
    "Arc",
    "Rc",
    "Weak",
    "Cow",
    "Cell",
    "RefCell",
    "Mutex",
    "RwLock",
    "OnceCell",
    "OnceLock",
    "PhantomData",
    "Pin",
    "NonNull",
    "Sender",
    "Receiver",
    "SyncSender",
    "UnboundedSender",
    "UnboundedReceiver",
    "Rx",
    "Tx",
    "JoinHandle",
    "JoinSet",
    "Task",
    "Future",
    "Stream",
    "Sink",
    "Iter",
    "IntoIter",
    "Range",
    "Reverse",
    "Wrapping",
    "Saturating",
    "Poll",
    "Ready",
    "Pending",
    "Duration",
];

/// Smart pointers and locks whose contents a method call reaches —
/// through `Deref`, or through the guard `lock()`/`read()`/`borrow()`
/// hands back. A parameter typed as one of these is, for resolving its
/// methods, the type inside. All std except `ArcSwap`, which is a library
/// type (the `arc-swap` crate) listed for the same reason.
pub const SMART_POINTERS_AND_LOCKS: &[&str] = &[
    "Arc", "Rc", "Box", "Pin", "Mutex", "RwLock", "RefCell", "Cell", "ArcSwap",
];

/// Wrappers a value is constructed through (`Arc::new(Inner::new())`)
/// that forward every method call to their contents via `Deref` or a
/// guard, so a local built through one is typed as the contents.
pub const DEREF_TRANSPARENT_WRAPPERS: &[&str] = &[
    "Arc", "Rc", "Box", "RefCell", "Cell", "Mutex", "RwLock", "Pin", "Cow",
];

/// The wrappers a `self.<field>` type is seen through: those whose
/// contents a direct method call reaches. `Option`/`Vec` are absent on
/// purpose — `self.opt.m()` is the container's own method or an
/// extension trait implemented for the container.
pub const FIELD_WRAPPERS: &[&str] = &["Arc", "Rc", "Box", "RefCell", "Mutex", "RwLock"];

/// Wrappers whose guard (`lock()`/`write()`/`borrow()`) dereferences to
/// the contents.
pub const LOCK_WRAPPERS: &[&str] = &["Mutex", "RwLock", "RefCell"];

/// Single-argument wrappers a function's declared return type is seen
/// through when asking which type the call produces: `Option<T>`,
/// pointers and locks around `T`. Any type named `…Result` (`Result`,
/// `io::Result`, `anyhow::Result`) is one too, see [`is_return_wrapper`].
pub const RETURN_WRAPPERS: &[&str] =
    &["Option", "Box", "Arc", "Rc", "Pin", "Cow", "Mutex", "RwLock"];

/// Whether `name` (a bare type name) wraps the type a function returns:
/// one of [`RETURN_WRAPPERS`] or a `Result` alias.
pub fn is_return_wrapper(name: &str) -> bool {
    name.ends_with("Result") || RETURN_WRAPPERS.contains(&name)
}

/// `Outer<Inner>` -> `(Outer, Inner)` when the hint has exactly one plain
/// type argument (a path whose last segment is capitalised, no lifetimes,
/// no nested generics) and `Outer` is not in [`NOT_DEREF_TO_ARG`]. `Outer`
/// keeps any path prefix (`web::Data`).
pub fn deref_wrapper_split(ty: &str) -> Option<(&str, &str)> {
    let ty = ty.trim();
    let open = ty.find('<')?;
    let inner = ty.strip_suffix('>')?.get(open + 1..)?.trim();
    let outer = ty[..open].trim();
    let outer_last = outer.rsplit("::").next().unwrap_or(outer);
    if outer_last.is_empty()
        || !outer_last.starts_with(char::is_uppercase)
        || NOT_DEREF_TO_ARG.contains(&outer_last)
    {
        return None;
    }
    let plain = !inner.is_empty()
        && inner
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
        && inner
            .rsplit("::")
            .next()
            .is_some_and(|l| l.starts_with(char::is_uppercase));
    plain.then_some((outer, inner))
}

#[cfg(test)]
mod tests {
    use super::deref_wrapper_split as split;

    #[test]
    fn splits_only_single_plain_arguments_of_deref_style_wrappers() {
        assert_eq!(split("State<AppState>"), Some(("State", "AppState")));
        assert_eq!(
            split("web::Data<app::State>"),
            Some(("web::Data", "app::State"))
        );
        assert_eq!(split("Vec<Foo>"), None);
        assert_eq!(split("Sender<Msg>"), None);
        assert_eq!(split("Store<K, V>"), None);
        assert_eq!(split("Ref<'a, Foo>"), None);
        assert_eq!(split("Wrap<u32>"), None);
        assert_eq!(split("Plain"), None);
    }
}
