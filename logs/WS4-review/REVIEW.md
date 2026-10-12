# WS4 orchestrator review of ab/WS4-v5 (2026-10-12 ~15:00)

Verdict: NOT committable yet. The A4/A5/A6 direction is right and most drops
verified correct (Java bare→unrelated-class methods, Python builtins→methods,
PHP/Perl/Rust/C# bare→method, Go cross-package bare, external-typed Go
receivers, JS/TS decoys, calcom/4minitz test-mock decoys). But Step 0c refuses
far too eagerly, and the qn-collision change adds wrong fan-out in languages
outside the brief. Classification scripts: logs/WS4-review/{classify,pair,reasons,unres}.py.

## Principle to apply (all four Step-0c languages)
Step 0c may REFUSE (`not-in-imported-module`) only when the module lookup is
decisive: the receiver IS the module (`mod.f()`, `import {f}; f()`), or a path
ending in a type/class/function the module itself provides, with that type's
method set known. When the path passes through a module-level VALUE (variable,
instance, runtime-built export, property), through a function whose result type
is unknown / qualified by another package / external, or through a chain whose
intermediate return type is unknown, return None so the pre-WS4 steps apply
(duck typing at medium is the established behaviour for an unknown receiver;
it is not the A5 decoy, which is `mod.f()` binding to an `f` in a different
module).

## Regressions (correct edges lost unless noted)
R1 Go: self-shadowing short var decl. `x, err := x.F(...)`: the new variable is
   treated as in scope inside its own RHS (Go spec: scope starts AFTER the
   ShortVarDecl/VarSpec). ~2,200 edges. ghcli pkg/cmd/attestation/verify/
   attestation_test.go:55 (`artifact, err := artifact.NewDigestedArtifact(...)`),
   casdoor object/provider.go:366/395/401/431 (`pp, err := pp.New…()`).
R2 Module-level VALUE receivers refused, all four langs:
   Go `pkg.Var.m()` treated as `pkg.Type.m()` → refuse: photoprism
   internal/api/cache.go:128 `ttl.CacheDefault.Int()` (→ ttl.Duration.Int),
   internal/api/api_auth.go:91 `acl.Rules.DenyAll` (→ acl.ACL.DenyAll), casdoor
   controllers/auth.go:224 `object.DeviceAuthMap.LoadAndDelete` (interface + 3
   impls, all in the module). Python: bugbug libs/hackbot-runtime/tests/
   test_anthropic_wif.py:17 `anthropic_wif._refresher.stop()`; dailynotes
   app/routes.py:941 `email_service.send_password_reset` (module-level
   instance); saleor 201 of this shape. JS: ghost
   services/stripe/stripe-service.js:134 `memberWelcomeEmailService.api.m()`,
   boot.js:442 `emailService.service.m()`. TS: metabase
   EntityPickerModal.tsx:138 `resizeObserver.subscribe` (default export is
   `createResizeObserver()`).
R3 Go `v := pkg.F(); v.m()`: method looked up in pkg instead of F's declared
   result type. Result type qualified by another package (`idv.IdvProvider`),
   external, or unknown → must not be `not-in-imported-module` under pkg
   (resolve the qualifier through F's file imports, external → external
   bucket, unknown → fall through). casdoor controllers/user.go:990
   `idvProvider.VerifyIdentity`; ghcli pkg/cmd/api/api_test.go:439/452
   `recorder.Record/Finish`; casdoor object/storage.go:152. Also: go.rs does
   not record embedded-struct promotion, so a `T.m` miss on a workspace T is
   NOT proof of absence — fall through, do not refuse.
R4 Chains off constructed/module members refused: calcom packages/features/
   bookings/lib/service/RegularBookingService.ts:1363
   `new CalendarEventBuilder({...}).withX(...)...build()` (builder methods
   returning this) all refused.
R5 TS/JS module resolution failures (the module DOES define the name):
   (a) Deno root-relative default import: bewcloud pages/calendar.ts:84
       `import Loading from '/components/Loading.ts'` + `export default
       function Loading()` → refused (also pages/settings.ts:286 Settings).
   (b) calcom workspace package subpaths: apps/web/modules/apps/components/
       AppCard.tsx:131 `import { markdownToSafeHTML } from
       "@calcom/lib/markdownToSafeHTML"` (packages/lib/markdownToSafeHTML.ts:10
       `export function markdownToSafeHTML`) → refused; same for getSlots
       (@calcom/features/schedules/lib/slots), getUTCOffsetByTimezone /
       formatTime (@calcom/lib/dayjs → packages/lib/dayjs/index.ts).
   (c) Alias matched by suffix picks the wrong copy: metabase
       frontend/src/embedding-sdk-bundle/components/public/question/
       shared-tests/card-prop.spec.tsx:113 `__support__/ui` → nearest copy
       frontend/src/embedding-sdk-bundle/test/__support__/ui.tsx, while
       tsconfig maps it to frontend/test/__support__/ui.tsx (defines
       mockGetBoundingClientRect). Read tsconfig `paths` TARGETS (manifest.rs
       already reads the keys), or when an alias matches several files look
       the name up in all of them and refuse only if none provides it.
   (d) Static method inherited through an imported class: adocasts
       app/dtos/blog_list.ts:38 `AssetDto.fromModel(...)` (declared on base
       BaseModelDto) → refused; must walk bases.
   Scale: TS refusals whose dropped target sits in the module's own file:
   1,335 (metabase 887, calcom 343, expensify 68); JS 143. Each class must be
   fixed or explained.
R6 Go same-package calls refused as `candidates-in-other-files`, both inside
   `t.Run(…, func(t){…})` closures: ghcli pkg/cmd/workflow/list/list_test.go:218
   `listRun(tt.opts)` (list.go same dir, both `package list`), repo/edit/
   edit_test.go:145/329; photoprism internal/config/config_auth_test.go:218
   `c := NewMinimalTestConfig(...); c.KeysPath()` (1 candidate, same dir).
R7 Go generic receiver methods lose their owner: `func (m *Map[K, V])
   ReplaceKey` is extracted as `ordered.ReplaceKey`, so the now-typed receiver
   refuses: photoprism pkg/list/ordered/map_test.go:110. Strip type params from
   the receiver type in go.rs (structural).
R8 qn-collision change outside the brief (WRONG edges added):
   Erlang +7,185 add-only, all erlang-otp: lib/tools/src/xref_base.erl:1008
   `filter(...)` (`-import(lists, …)`) → `lists1:filter` in debugger test data
   — a different module. OCaml +10,074 / −4,399: local names fan out to every
   same-named top-level let (src/fiber/src/core.ml:277 `k x` → 3 unrelated
   `k`; src/dune_vcs/vcs.ml:156 `sprintf` → stdune config/console/global_lock).
   Dart: `matcher.expect(...)` (external prefix) → workspace `expect`s.
   Some changes are improvements (Scala logError → internal/Logging.scala,
   F# SearchComponent.Main, Zig assert → lib/std/debug.zig). Rule: a qn
   collision with no scoping evidence must not fan out to every copy; prefer
   caller file / same dir / imported module, else refuse as ambiguous
   (recorded). Never bind a call to a callable of a different module name.
   Classify Zig/OCaml/Erlang/Scala/F#/Dart adds+drops after the fix.

## Verified OK (no action)
Java bare→unrelated classes (external base, e.g. Hibernate/Shiro subclasses);
Python `filter()`/`open()`→methods; PHP builtins→methods; Rust `update`
(diesel) / `eq` / `post()` → methods; Perl `Pkg->m` exact; C# bare→unrelated;
Go external-typed receivers (fiber Ctx/App); JS/TS calcom/4minitz test-mock
decoys; 681 low framework-entry edges in expensify bundled actions (event
handler registrations, acceptable).
