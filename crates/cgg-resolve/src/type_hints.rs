//! Lightweight type propagation for receiver-hint rewriting.
//!
//! Scans each file's definitions and references to infer variable types
//! from:
//! 1. **Parameter type annotations** — `fn foo(x: Service)` means `x`
//!    has type `Service` inside `foo`.
//! 2. **Constructor assignments** — `let x = Foo::new()`, `x = Foo()`,
//!    `var x = new Foo()` means `x` has type `Foo`.
//! 3. **Typed variable declarations** — `Foo x = ...` (Java/C#/C++).
//!
//! The output is a rewritten set of `RefRecord`s where `receiver_hint`
//! has been replaced with the inferred type name when possible. This
//! lets the intra-file linker match `x.method()` against
//! `Foo::method` / `Foo.method`.

use cgg_core::{DefRecord, FileFacts};
use std::collections::HashMap;

/// `return_types`, pre-indexed by the lowercased return type.
///
/// Strategy 4 asks, for one receiver name, "which functions return a
/// type whose lowercased name equals this receiver, or equals it minus
/// a trailing `s`". Answering that by walking the whole map is
/// O(receivers x map), and the map is corpus-wide: on `erlang-otp` that
/// was **817,607,605 iterations**, each allocating a `to_lowercase()`
/// and a `format!`. The lowercasing depends only on the map, so it is
/// hoisted here and done once per run instead of once per receiver.
///
/// Keys are the lowercased return type. A receiver `configs` finds
/// exact matches under `configs` and plural matches under `config`,
/// which is the same two-way test the filter did inline.
#[derive(Debug, Default)]
pub struct ReturnTypeIndex<'a> {
    by_lower_ret: HashMap<String, Vec<(&'a str, &'a str)>>,
    empty: bool,
}

impl<'a> ReturnTypeIndex<'a> {
    pub fn build(return_types: &HashMap<&'a str, &'a str>) -> Self {
        let mut by_lower_ret: HashMap<String, Vec<(&'a str, &'a str)>> = HashMap::new();
        for (&f, &r) in return_types {
            by_lower_ret
                .entry(r.to_lowercase())
                .or_default()
                .push((f, r));
        }
        // Sorted once, here, rather than per lookup. The old code sorted
        // each candidate list by function name after filtering; function
        // names are unique in the map, so sorting each bucket once gives
        // every lookup the same order it used to compute.
        for v in by_lower_ret.values_mut() {
            v.sort_unstable_by(|a, b| a.0.cmp(b.0));
        }
        Self {
            empty: return_types.is_empty(),
            by_lower_ret,
        }
    }

    fn is_empty(&self) -> bool {
        self.empty
    }

    /// Candidates for `rh_lower`, in the exact order the old inline
    /// filter-then-sort produced: exact matches first (sorted by
    /// function name), then plural matches (likewise).
    ///
    /// The old comparator was `exact(a).cmp(&exact(b)).then(a.0.cmp(b.0))`
    /// where `exact` is false for an exact match, so exact sorted before
    /// plural and ties broke on the function name. Two pre-sorted buckets
    /// concatenated in that order reproduce it exactly, and a candidate
    /// cannot land in both: that would need `rl == rh_lower` and
    /// `rl + "s" == rh_lower` simultaneously.
    fn candidates(&self, rh_lower: &str) -> impl Iterator<Item = (&'a str, &'a str)> {
        let exact = self.by_lower_ret.get(rh_lower);
        let plural = rh_lower
            .strip_suffix('s')
            .and_then(|stem| self.by_lower_ret.get(stem));
        exact
            .into_iter()
            .flatten()
            .chain(plural.into_iter().flatten())
            .copied()
    }
}

/// Rewrite receiver hints in-place using inferred type information.
pub fn propagate_types(facts: &mut FileFacts) {
    propagate_types_with_returns(facts, &ReturnTypeIndex::default());
}

/// Build a map of function_simple_name -> return_type from all
/// definitions across all files. Parses return types from signature_hint.
pub fn build_return_type_map<'a>(
    all_facts: &'a [FileFacts],
) -> HashMap<&'a str, &'a str> {
    let mut map: HashMap<&'a str, &'a str> = HashMap::new();
    for facts in all_facts {
        for def in &facts.definitions {
            if let Some(ret) = extract_return_type(&def.signature_hint)
                && ret.starts_with(char::is_uppercase)
                && !is_primitive(ret)
            {
                map.entry(def.simple_name.as_str()).or_insert(ret);
            }
        }
    }
    map
}

/// Per-language tally names for the type propagator.
///
/// `profile::count` takes a `&'static str`, so attributing a count to a
/// language needs a fixed table rather than a formatted name — the
/// alternative is leaking a string per language, which `docs-check.py`
/// check 9 exists to prevent.
///
/// The list is the languages that actually appear in the corpus repos
/// under investigation plus the common ones; anything else shares
/// `other::`. A catch-all is fine for "is this one language's problem"
/// right up until the catch-all *is* the answer, which is what happened
/// on the first pass here: `erlang-otp` put 1.86 billion inner-scan
/// steps in `types::`, and only naming C and C++ showed they were the
/// ones spending it, not Erlang.
struct LangTallies {
    refs: &'static str,
    defs: &'static str,
    enclosing_scans: &'static str,
    s4_entered: &'static str,
    s4_inner_scan: &'static str,
}

macro_rules! lang_tallies_table {
    ($($lang:literal),* $(,)?) => {
        fn lang_tallies(lang: &str) -> LangTallies {
            match lang {
                $(
                    $lang => LangTallies {
                        refs: concat!($lang, "::refs"),
                        defs: concat!($lang, "::defs"),
                        enclosing_scans: concat!($lang, "::enclosing-def-steps"),
                        s4_entered: concat!($lang, "::s4-entered"),
                        s4_inner_scan: concat!($lang, "::s4-inner-scan-steps"),
                    },
                )*
                _ => LangTallies {
                    refs: "other::refs",
                    defs: "other::defs",
                    enclosing_scans: "other::enclosing-def-steps",
                    s4_entered: "other::s4-entered",
                    s4_inner_scan: "other::s4-inner-scan-steps",
                },
            }
        }
    };
}

lang_tallies_table!(
    "erlang",
    "c",
    "cpp",
    "rust",
    "python",
    "java",
    "javascript",
    "typescript",
    "go",
    "perl",
    "elixir",
    "bash",
    "ruby",
    "php",
    "csharp",
    "kotlin",
    "swift",
);

/// Rewrite receiver hints using both local type info and a global
/// return-type map built from all files' definitions.
pub fn propagate_types_with_returns(
    facts: &mut FileFacts,
    return_types: &ReturnTypeIndex<'_>,
) {
    // Pass 1: Extract type hints from definition signatures.
    //
    // The names and types are slices of `def.signature_hint`, and `facts`
    // is mutably borrowed at the end of this function, so the map cannot
    // borrow them directly. They go into an owned store that this function
    // drops instead.
    //
    // That store is why there is no `leak_str` here any more. It used to
    // `Box::leak` a copy of every parameter name and type, justified in a
    // comment by "we're in a short-lived analysis pass" — true while the
    // pipeline was a private function in a binary that analyzed once and
    // exited, and false from 0.6.0, when `cgg::analyze` became callable in
    // a loop by a library, a Python module and a C ABI. Measured at ~161
    // bytes per call on cgg's own tree, and it accumulated forever in a
    // long-lived host process. The allocation *count* is unchanged — the
    // leaked version called `to_string()` too — so this costs nothing; the
    // strings are simply freed now.
    let param_store: Vec<(u32, String, String)> = facts
        .definitions
        .iter()
        .flat_map(collect_param_types)
        .collect();
    // Key: (enclosing_start_byte, variable_name) -> type_name. Last write
    // wins, exactly as the repeated `map.insert` did.
    let type_map: HashMap<(u32, &str), &str> = param_store
        .iter()
        .map(|(byte, name, ty)| ((*byte, name.as_str()), ty.as_str()))
        .collect();

    // Pass 2: Scan references for constructor patterns that reveal types.
    // We look for assignment-like patterns in the source by examining
    // refs that look like constructors (name matches a type pattern).
    let constructor_types = find_constructor_assignments(facts);

    // Pass 2b: Build map from explicit local variable type declarations,
    // keyed by (enclosing-callable start_byte, var_name) so two `let
    // builder = XBuilder::new()` in different functions of the same file
    // don't conflate to one type — a file-wide last-write-wins map
    // mis-resolves `builder.method()` to whichever builder type was
    // declared last in the file.
    let mut local_type_map: HashMap<&str, &str> = HashMap::new();
    // Scoped lookup for self-field LocalTypes: keyed by the method's
    // body start_byte so we don't bleed Type A's `self.store` into
    // Type B's methods within the same file. Built from any LocalType
    // whose var_name starts with `self.`.
    let mut self_field_map: HashMap<(u32, &str), &str> = HashMap::new();
    for lt in &facts.local_types {
        if lt.var_name.starts_with("self.") {
            self_field_map
                .insert((lt.scope_byte, lt.var_name.as_str()), lt.type_name.as_str());
        } else {
            local_type_map.insert(lt.var_name.as_str(), lt.type_name.as_str());
        }
    }

    // Pass 2c: earliest bare call site per function name — built on
    // first use, not up front.
    //
    // Strategy 4 needs "is `fn_name` called, with no receiver, before
    // this site". That was `facts.references.iter().any(...)` per
    // candidate, so the cost was O(receivers x candidates x references)
    // — **1,864,725,063 steps** on `erlang-otp`, 93% of them from its
    // 171 vendored C++ files. The predicate only ever asks whether the
    // *earliest* such call precedes the site, so one pass over the
    // references answers every later question in O(1).
    //
    // `OnceCell`, because most files never reach Strategy 4 at all: it
    // needs a non-empty return-type map, a receiver that survives every
    // filter above, and strategies 1 and 3 to have missed. Building this
    // unconditionally charged an O(references) pass plus a map
    // allocation to every file in every language to fix a cost that only
    // some of them pay. Measured: it made `asyncapi-spec` **+82.6%**,
    // `firebase-samples` +59.9% and `cpp-nlohmann-json` +21.5% while the
    // corpus total still read -8.9%, which is how a real regression
    // hides inside a good average.
    let called_before: std::cell::OnceCell<HashMap<&str, u32>> =
        std::cell::OnceCell::new();
    let build_called_before = || {
        let mut m: HashMap<&str, u32> = HashMap::new();
        for r in &facts.references {
            if r.receiver_hint.is_empty() {
                m.entry(r.name.as_str())
                    .and_modify(|b| *b = (*b).min(r.site_byte))
                    .or_insert(r.site_byte);
            }
        }
        m
    };

    // Pass 3: Rewrite receiver_hints.
    let t = lang_tallies(&facts.language);
    cgg_core::profile::count(t.refs, facts.references.len() as u64);
    cgg_core::profile::count(t.defs, facts.definitions.len() as u64);
    let _s3 = cgg_core::profile::span("types::pass3");
    let mut rewrites: Vec<(usize, String)> = Vec::new();
    for (i, rref) in facts.references.iter().enumerate() {
        let rh = rref.receiver_hint.as_str();
        if rh.is_empty()
            || rh == "self"
            || rh == "Self"
            || rh == "cls"
            || rh == "this"
            || rh == cgg_core::VALUE_REF_HINT
        {
            continue;
        }

        // Special-case `self.<field>` BEFORE the dot/colon filter
        // below — the field's type comes from the per-method scoped
        // self_field_map populated by the Rust extractor.
        if rh.starts_with("self.") {
            if let Some(enc) = enclosing_def(facts, rref.site_byte)
                && let Some(&ty) = self_field_map.get(&(enc.start_byte, rh))
            {
                rewrites.push((i, ty.to_string()));
                continue;
            }
            // No match — leave as-is so the resolver can still try a
            // direct lookup downstream.
            continue;
        }

        if rh.starts_with(char::is_uppercase) || rh.contains("::") || rh.contains('.') {
            continue;
        }

        cgg_core::profile::count(t.enclosing_scans, facts.definitions.len() as u64);
        let enclosing = {
            let _s = cgg_core::profile::span("types::enclosing-def");
            enclosing_def(facts, rref.site_byte)
        };

        // Strategy 1: parameter type annotations
        if let Some(enc) = enclosing
            && let Some(&ty) = type_map.get(&(enc.start_byte, rh))
        {
            rewrites.push((i, ty.to_string()));
            continue;
        }

        // Strategy 3: explicit local variable type declarations
        if let Some(&ty) = local_type_map.get(rh) {
            rewrites.push((i, ty.to_string()));
            continue;
        }

        // Strategy 4: return-type inference. If the receiver variable
        // was assigned from a function call whose return type we know,
        // use that. We check if any ref in this file is a bare call
        // to a function with a known return type, appearing before
        // this ref, and the ref's name matches our receiver.
        // Simplified: just check if receiver_hint matches a known
        // function name's return type (covers `let x = getService(); x.run()`)
        if !return_types.is_empty() {
            let _s4 = cgg_core::profile::span("types::strategy4");
            cgg_core::profile::count(t.s4_entered, 1);
            // Check if there's a ref earlier in this file that calls
            // a function whose return type matches. We use a heuristic:
            // if the variable name is a common derivative of the return
            // type (e.g., "service" from "Service", "config" from "Config")
            // OR if we find a bare call to a function returning that type.
            let rh_lower = rh.to_lowercase();
            // The order here is load-bearing, not tidiness. The match is
            // deliberately loose — it accepts an exact name match OR a
            // plural, so `Config` and `Configs` BOTH claim a receiver
            // called `configs`. Taking whichever came first out of hash
            // order meant a 20-line file produced two different graphs
            // across 25 identical single-threaded runs with default
            // flags. `ReturnTypeIndex::candidates` yields exact matches
            // before plural ones, each bucket sorted by function name,
            // which is the order the old filter-then-sort computed.
            for (fn_name, ret_type) in return_types.candidates(&rh_lower) {
                cgg_core::profile::count(t.s4_inner_scan, 1);
                // Verify this function is actually called in this scope.
                let called = called_before
                    .get_or_init(build_called_before)
                    .get(fn_name)
                    .is_some_and(|&b| b < rref.site_byte);
                if called {
                    rewrites.push((i, ret_type.to_string()));
                    break;
                }
            }
            if rewrites.last().map(|(idx, _)| *idx) == Some(i) {
                continue;
            }
        }

        // Strategy 2: constructor/lowercase heuristic
        if let Some(ty) = constructor_types.get(rh) {
            rewrites.push((i, ty.clone()));
        }
    }
    for (i, ty) in rewrites {
        facts.references[i].receiver_hint = ty;
    }
}

/// Parameter `(enclosing_start_byte, name, type)` triples from one
/// definition's `signature_hint`.
///
/// Returns owned strings rather than writing borrowed ones into a map: the
/// caller needs them to outlive its borrow of `facts`, and owning them
/// there is what lets the caller free them. See the call site.
fn collect_param_types(def: &DefRecord) -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    // Parse parameter types from signature_hint.
    // Patterns we recognize:
    //   Rust:   `fn foo(x: Service, y: &Helper)`
    //   Python: `def foo(self, x: Service, y: Helper):`
    //   Java:   `public void foo(Service x, Helper y)`
    //   TS:     `foo(x: Service, y: Helper)`
    //   Go:     `func foo(x Service, y *Helper)`
    //   Kotlin: `fun foo(x: Service, y: Helper)`
    //   C#:     `void Foo(Service x, Helper y)`
    let sig = &def.signature_hint;
    if sig.is_empty() {
        return out;
    }

    // Find the parameter list between parens
    let Some(open) = sig.find('(') else {
        return out;
    };
    let Some(close) = sig.rfind(')') else {
        return out;
    };
    if close <= open {
        return out;
    }
    let params_str = &sig[open + 1..close];

    for param in params_str.split(',') {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }

        // Try "name: Type" pattern (Rust, Python, TS, Kotlin)
        if let Some((name, ty)) = parse_colon_param(param) {
            out.push((def.start_byte, name.to_string(), ty.to_string()));
            continue;
        }

        // Try "Type name" pattern (Java, C#, C++, Go)
        if let Some((name, ty)) = parse_type_first_param(param) {
            out.push((def.start_byte, name.to_string(), ty.to_string()));
        }
    }
    out
}

fn parse_colon_param(param: &str) -> Option<(&str, &str)> {
    // "x: Service" or "x: &Service" or "x: *Service"
    let (name, rest) = param.split_once(':')?;
    let name = name
        .trim()
        .trim_start_matches("mut ")
        .trim()
        .rsplit(' ')
        .next()
        .unwrap_or(name.trim());
    let ty = rest
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches('*')
        .trim();
    // Take just the type identifier (before any <, [, etc.)
    let ty = ty.split(['<', '[', ',', ')']).next().unwrap_or(ty).trim();
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    // Skip primitive types
    if is_primitive(ty) {
        return None;
    }
    Some((name, ty))
}

fn parse_type_first_param(param: &str) -> Option<(&str, &str)> {
    // "Service x" or "final Service x" or "Service<T> x"
    let parts: Vec<&str> = param.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    // Skip modifiers
    let (ty_idx, name_idx) = if matches!(parts[0], "final" | "const" | "var" | "val") {
        if parts.len() < 3 {
            return None;
        }
        (1, 2)
    } else {
        (0, parts.len() - 1)
    };
    let ty = parts[ty_idx].trim_end_matches(['<', '>']);
    let name = parts[name_idx];
    if ty.is_empty() || name.is_empty() {
        return None;
    }
    if !ty.starts_with(char::is_uppercase) {
        return None;
    }
    if is_primitive(ty) {
        return None;
    }
    Some((name, ty))
}

fn find_constructor_assignments(facts: &FileFacts) -> HashMap<String, String> {
    // Look for refs that are constructor calls and try to find the
    // variable they're assigned to. We use a heuristic: if a RefRecord
    // has no receiver_hint and its name starts with uppercase (looks
    // like a type), it's likely a constructor call. We then look for
    // other refs in the same function that use that type name as a
    // receiver.
    //
    // Actually, we can do better: scan the definitions for constructor
    // variants and map their simple_name to qualified_name prefix.
    // Then for any ref whose name matches a type, we know that variable
    // assignments of that type exist.
    //
    // Simplest approach: collect all type names from definitions (class
    // names = any def whose qualified_name has the type as a segment).
    let mut type_names: std::collections::HashSet<&str> =
        std::collections::HashSet::new();
    for d in &facts.definitions {
        // Each segment of the qualified name that starts with uppercase
        // is likely a type name.
        for seg in d.qualified_name.split([':', '.']) {
            if seg.starts_with(char::is_uppercase) && !seg.is_empty() {
                type_names.insert(seg);
            }
        }
    }

    // Now scan refs: if a ref has name matching a type_name and no
    // receiver (bare call like `Foo()` or `new Foo()`), it's a
    // constructor. We can't easily find the variable name from the
    // AST at this point (we only have RefRecords), so we rely on
    // the parameter-type approach for most cases.
    //
    // For the common pattern where the variable name matches the type
    // (lowercased), we can infer: `service.run()` -> type `Service`.
    let mut map = HashMap::new();
    // Sorted. `type_names` is a HashSet and the inserts below are
    // last-write-wins, so two type names whose lowercase forms collide
    // (`HttpClient`/`HTTPClient`, `Repo`/`REPO`) fought for the same key
    // and the winner depended on hash order.
    let mut type_names_sorted: Vec<&&str> = type_names.iter().collect();
    type_names_sorted.sort_unstable();
    for ty in type_names_sorted {
        // `ty[..1]` panics whenever the first character is multi-byte,
        // and identifiers may legally be non-ASCII in Python, Java, C#
        // and Rust. Split on the first *character* instead of the first
        // byte.
        let mut chars = ty.chars();
        let lower = match chars.next() {
            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
            None => continue,
        };
        map.insert(lower, ty.to_string());
        // Also try full lowercase
        map.insert(ty.to_lowercase(), ty.to_string());
    }
    map
}

fn enclosing_def(facts: &FileFacts, byte: u32) -> Option<&DefRecord> {
    let mut best: Option<(&DefRecord, u32)> = None;
    for d in &facts.definitions {
        if d.start_byte <= byte && byte < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((d, span)),
                Some((_, b)) if span < b => best = Some((d, span)),
                _ => {}
            }
        }
    }
    best.map(|(d, _)| d)
}

fn extract_return_type(sig: &str) -> Option<&str> {
    // Rust: `fn foo() -> Config`
    if let Some(pos) = sig.find("->") {
        let ret = sig[pos + 2..].trim();
        let ret = ret
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim();
        let ret = ret.split(['<', '{', ',', ' ']).next().unwrap_or(ret).trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // Java/C#/Go: return type is before the function name
    // `public Service getService()` or `func GetConfig() Config`
    // TS/Kotlin: `fun foo(): Config` or `foo(): Config`
    if let Some(pos) = sig.find("): ") {
        let ret = sig[pos + 3..].trim();
        let ret = ret.split(['<', '{', ' ', '?']).next().unwrap_or(ret).trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // Go: `func Foo() Config {` — return type after ) and before {
    if let Some(paren_close) = sig.rfind(')') {
        let after = sig[paren_close + 1..].trim();
        let after = after.trim_start_matches('*');
        let ret = after.split(['{', ',', ' ']).next().unwrap_or("").trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) && !is_primitive(ret) {
            return Some(ret);
        }
    }
    None
}

fn is_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "int"
            | "i32"
            | "i64"
            | "u32"
            | "u64"
            | "f32"
            | "f64"
            | "bool"
            | "str"
            | "String"
            | "string"
            | "void"
            | "char"
            | "byte"
            | "short"
            | "long"
            | "float"
            | "double"
            | "usize"
            | "isize"
            | "u8"
            | "i8"
            | "u16"
            | "i16"
            | "number"
            | "boolean"
            | "any"
            | "object"
            | "Int"
            | "Long"
            | "Float"
            | "Double"
            | "Boolean"
            | "Unit"
            | "Nothing"
            | "Void"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use cgg_core::{DefVariant, RefRecord};
    use std::path::PathBuf;

    fn mk_facts(defs: Vec<DefRecord>, refs: Vec<RefRecord>) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from("/tmp/test.rs"), "rust");
        f.definitions = defs;
        f.references = refs;
        f
    }

    fn mk_def(qn: &str, sig: &str, start: u32, end: u32) -> DefRecord {
        DefRecord {
            simple_name: qn.rsplit("::").next().unwrap_or(qn).to_string(),
            qualified_name: qn.to_string(),
            variant: DefVariant::FreeFunction,
            start_line: 1,
            end_line: 10,
            start_byte: start,
            end_byte: end,
            signature_hint: sig.to_string(),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn param_type_rewrites_receiver() {
        let defs = vec![
            mk_def("Foo::run", "fn run(&self)", 0, 100),
            mk_def("main", "fn main(svc: Service)", 100, 200),
        ];
        let refs = vec![RefRecord {
            name: "run".into(),
            receiver_hint: "svc".into(),
            site_line: 5,
            site_byte: 150,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Service");
    }

    #[test]
    fn java_style_type_first_param() {
        let defs = vec![
            mk_def("Helper.add", "public int add(int a)", 0, 50),
            mk_def("Main.run", "public void run(Helper h)", 50, 150),
        ];
        let refs = vec![RefRecord {
            name: "add".into(),
            receiver_hint: "h".into(),
            site_line: 3,
            site_byte: 100,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Helper");
    }

    #[test]
    fn lowercase_variable_matches_type() {
        let defs = vec![
            mk_def("Service.run", "fn run(&self)", 0, 50),
            mk_def("main", "fn main()", 50, 150),
        ];
        let refs = vec![RefRecord {
            name: "run".into(),
            receiver_hint: "service".into(),
            site_line: 3,
            site_byte: 100,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Service");
    }

    #[test]
    fn uppercase_receiver_not_rewritten() {
        let defs = vec![mk_def("Foo.bar", "fn bar()", 0, 50)];
        let refs = vec![RefRecord {
            name: "bar".into(),
            receiver_hint: "Foo".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        // Already uppercase — should not be touched
        assert_eq!(facts.references[0].receiver_hint, "Foo");
    }

    #[test]
    fn self_not_rewritten() {
        let defs = vec![mk_def("Foo.bar", "fn bar(&self)", 0, 50)];
        let refs = vec![RefRecord {
            name: "baz".into(),
            receiver_hint: "self".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "self");
    }
}
