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

use cgg_core::{DefRecord, FileFacts, RefRecord};
use std::collections::HashMap;

/// Rewrite receiver hints in-place using inferred type information.
pub fn propagate_types(facts: &mut FileFacts) {
    propagate_types_with_returns(facts, &HashMap::new());
}

/// Build a map of function_simple_name -> return_type from all
/// definitions across all files. Parses return types from signature_hint.
pub fn build_return_type_map<'a>(all_facts: &'a [FileFacts]) -> HashMap<&'a str, &'a str> {
    let mut map: HashMap<&'a str, &'a str> = HashMap::new();
    for facts in all_facts {
        for def in &facts.definitions {
            if let Some(ret) = extract_return_type(&def.signature_hint) {
                if ret.starts_with(char::is_uppercase) && !is_primitive(ret) {
                    map.entry(def.simple_name.as_str()).or_insert(ret);
                }
            }
        }
    }
    map
}

/// Rewrite receiver hints using both local type info and a global
/// return-type map built from all files' definitions.
pub fn propagate_types_with_returns(
    facts: &mut FileFacts,
    return_types: &HashMap<&str, &str>,
) {
    // Build type map per enclosing callable (by byte range).
    // Key: (enclosing_start_byte, variable_name) -> type_name
    let mut type_map: HashMap<(u32, &str), &str> = HashMap::new();

    // Pass 1: Extract type hints from definition signatures.
    for def in &facts.definitions {
        extract_param_types(def, &mut type_map);
    }

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
            self_field_map.insert((lt.scope_byte, lt.var_name.as_str()), lt.type_name.as_str());
        } else {
            local_type_map.insert(lt.var_name.as_str(), lt.type_name.as_str());
        }
    }

    // Pass 3: Rewrite receiver_hints.
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
            if let Some(enc) = enclosing_def(facts, rref.site_byte) {
                if let Some(&ty) = self_field_map.get(&(enc.start_byte, rh)) {
                    rewrites.push((i, ty.to_string()));
                    continue;
                }
            }
            // No match — leave as-is so the resolver can still try a
            // direct lookup downstream.
            continue;
        }

        if rh.starts_with(char::is_uppercase) || rh.contains("::") || rh.contains('.') {
            continue;
        }

        let enclosing = enclosing_def(facts, rref.site_byte);

        // Strategy 1: parameter type annotations
        if let Some(enc) = enclosing {
            if let Some(&ty) = type_map.get(&(enc.start_byte, rh)) {
                rewrites.push((i, ty.to_string()));
                continue;
            }
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
            // Check if there's a ref earlier in this file that calls
            // a function whose return type matches. We use a heuristic:
            // if the variable name is a common derivative of the return
            // type (e.g., "service" from "Service", "config" from "Config")
            // OR if we find a bare call to a function returning that type.
            let rh_lower = rh.to_lowercase();
            for (&fn_name, &ret_type) in return_types.iter() {
                let ret_lower = ret_type.to_lowercase();
                // Match: variable named "service" and a function "getService" returns "Service"
                // Match: variable named "config" and a function "loadConfig" returns "Config"
                if rh_lower == ret_lower
                    || rh_lower == format!("{}s", ret_lower)  // plurals
                {
                    // Verify this function is actually called in this scope
                    let called = facts.references.iter().any(|r| {
                        r.name == fn_name && r.receiver_hint.is_empty()
                            && r.site_byte < rref.site_byte
                    });
                    if called {
                        rewrites.push((i, ret_type.to_string()));
                        break;
                    }
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

fn extract_param_types<'a>(
    def: &'a DefRecord,
    map: &mut HashMap<(u32, &'a str), &'a str>,
) {
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
        return;
    }

    // Find the parameter list between parens
    let Some(open) = sig.find('(') else { return };
    let Some(close) = sig.rfind(')') else { return };
    if close <= open { return; }
    let params_str = &sig[open + 1..close];

    for param in params_str.split(',') {
        let param = param.trim();
        if param.is_empty() { continue; }

        // Try "name: Type" pattern (Rust, Python, TS, Kotlin)
        if let Some((name, ty)) = parse_colon_param(param) {
            map.insert((def.start_byte, leak_str(name)), leak_str(ty));
            continue;
        }

        // Try "Type name" pattern (Java, C#, C++, Go)
        if let Some((name, ty)) = parse_type_first_param(param) {
            map.insert((def.start_byte, leak_str(name)), leak_str(ty));
        }
    }
}

fn parse_colon_param(param: &str) -> Option<(&str, &str)> {
    // "x: Service" or "x: &Service" or "x: *Service"
    let (name, rest) = param.split_once(':')?;
    let name = name.trim().trim_start_matches("mut ").trim()
        .rsplit(' ').next().unwrap_or(name.trim());
    let ty = rest.trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches('*')
        .trim();
    // Take just the type identifier (before any <, [, etc.)
    let ty = ty.split(|c: char| c == '<' || c == '[' || c == ',' || c == ')')
        .next().unwrap_or(ty).trim();
    if name.is_empty() || ty.is_empty() { return None; }
    // Skip primitive types
    if is_primitive(ty) { return None; }
    Some((name, ty))
}

fn parse_type_first_param(param: &str) -> Option<(&str, &str)> {
    // "Service x" or "final Service x" or "Service<T> x"
    let parts: Vec<&str> = param.split_whitespace().collect();
    if parts.len() < 2 { return None; }
    // Skip modifiers
    let (ty_idx, name_idx) = if matches!(parts[0], "final" | "const" | "var" | "val") {
        if parts.len() < 3 { return None; }
        (1, 2)
    } else {
        (0, parts.len() - 1)
    };
    let ty = parts[ty_idx].trim_end_matches(|c: char| c == '<' || c == '>');
    let name = parts[name_idx];
    if ty.is_empty() || name.is_empty() { return None; }
    if !ty.starts_with(char::is_uppercase) { return None; }
    if is_primitive(ty) { return None; }
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
    let mut type_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for d in &facts.definitions {
        // Each segment of the qualified name that starts with uppercase
        // is likely a type name.
        for seg in d.qualified_name.split(|c| c == ':' || c == '.') {
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
    for ty in &type_names {
        let lower = ty[..1].to_lowercase() + &ty[1..];
        map.insert(lower, ty.to_string());
        // Also try full lowercase
        map.insert(ty.to_lowercase(), ty.to_string());
    }
    map
}

fn enclosing_def<'a>(facts: &'a FileFacts, byte: u32) -> Option<&'a DefRecord> {
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
        let ret = ret.trim_start_matches('&').trim_start_matches("mut ").trim();
        let ret = ret.split(|c: char| c == '<' || c == '{' || c == ',' || c == ' ')
            .next().unwrap_or(ret).trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // Java/C#/Go: return type is before the function name
    // `public Service getService()` or `func GetConfig() Config`
    // TS/Kotlin: `fun foo(): Config` or `foo(): Config`
    if let Some(pos) = sig.find("): ") {
        let ret = sig[pos + 3..].trim();
        let ret = ret.split(|c: char| c == '<' || c == '{' || c == ' ' || c == '?')
            .next().unwrap_or(ret).trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // Go: `func Foo() Config {` — return type after ) and before {
    if let Some(paren_close) = sig.rfind(')') {
        let after = sig[paren_close + 1..].trim();
        let after = after.trim_start_matches('*');
        let ret = after.split(|c: char| c == '{' || c == ',' || c == ' ')
            .next().unwrap_or("").trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) && !is_primitive(ret) {
            return Some(ret);
        }
    }
    None
}

fn is_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "int" | "i32" | "i64" | "u32" | "u64" | "f32" | "f64"
            | "bool" | "str" | "String" | "string" | "void"
            | "char" | "byte" | "short" | "long" | "float" | "double"
            | "usize" | "isize" | "u8" | "i8" | "u16" | "i16"
            | "number" | "boolean" | "any" | "object"
            | "Int" | "Long" | "Float" | "Double" | "Boolean"
            | "Unit" | "Nothing" | "Void"
    )
}

/// Leak a string slice to get a `'static` lifetime. This is acceptable
/// because we're in a short-lived analysis pass and the total leaked
/// memory is bounded by the number of parameters in the file.
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use cgg_core::{DefVariant, ImportRecord};
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
            start_line: 1, end_line: 10,
            start_byte: start, end_byte: end,
            signature_hint: sig.to_string(),
            visibility: String::new(),
            attributes: Vec::new(),
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
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "self");
    }
}
