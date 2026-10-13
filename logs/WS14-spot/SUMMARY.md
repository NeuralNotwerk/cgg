# WS14 spot-check (2026-10-13 19:20) — 6 read-only auditors x 20 stratified changes (ab/WS14-v9)

119 judged (list2 agent skipped one entry, its indices >=13 are shifted by one): 106 correct, 13 regressions, 0 unclear.

## Correct edges LOST (fix in WS14b), with corpus prevalence from ab/WS14-v9
- Rust: pattern binding treated as in scope in its own initializer/scrutinee (`if let Some(k) = k(..)`): 22 sites (+27 other Rust shadowed-by-local to re-check).
- Zig: method calls on locals/params refused as candidates-in-other-files — local typed by a method's `?*T` return where T is a file-level @import alias; params named like a module (`canvas.rect`): 437 sites (44 were high).
- Perl: `key => $self->m` — key correctly no longer a call, but `$self->m` never binds to the enclosing package / @ISA sub: 291 sites (31 high).
- PHP: global function with several definitions and no include path from caller is refused (WordPress `__`, `wp_cache_get_multiple`; one copy is a `function_exists`-guarded fallback): 39 sites. Structural option: an unconditional definition beats a guarded fallback.

## Wrong -> wrong (no correct edge lost), fix in WS14b
- Scala base class/trait resolved by short name, ignoring the import (4 of 13: withTable, withSQLConf x2, checkAnswer via connect QueryTest).
- Scala companion `object X` members treated as inherited by `class/trait X`.
- Scala `this(...)` in an auxiliary constructor bound to the class's `def this` members.
- Kotlin scope-function lambda receiver ignored (`x.run { m() }` binds to a same-file override at high).
- OCaml non-recursive `let x = x ...` binds RHS to itself (known residual).
- Zig chained call on a generic type-constructor result (`std.array_list.Managed(T).init`) binds by bare name to a same-file `init`.

## Reason-label nits
external Scala names recorded as candidates-in-other-files instead of external-module (2).
