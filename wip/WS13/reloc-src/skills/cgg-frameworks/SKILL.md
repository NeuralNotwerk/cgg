---
name: cgg-frameworks
description: Teach `cgg` about a framework it does not recognise, so framework-invoked handlers stop showing up as unreferenced in `--dead-code`. Trigger when a cgg dead-code report flags route handlers, job tasks, lifecycle hooks, event listeners, actor message handlers, or ML `forward`/`step` methods that are obviously invoked by a framework; when the user says "these aren't dead, the framework calls them", "cgg doesn't understand our framework", "how do I tell cgg about X"; or when adding a house/internal framework to a project's cgg config. Also covers offering to contribute a rule for a *public* framework back upstream. Applies to Flask, FastAPI, Django, Express, NestJS, Spring, Quarkus, Gin, Rails, Sinatra, Laravel, Symfony, ASP.NET, Axum, Actix, Rocket, PyTorch, Celery, Quartz, MassTransit, Akka, Sidekiq and any in-house equivalent.
---

# cgg-frameworks — teaching cgg about a framework

A framework invokes user code by means that are not calls: a decorator
registers a route, a base class declares a contract the runtime calls, a
path string names a worker module. cgg resolves *calls*, so those entry
points have no caller and `--dead-code` reports them — and then reports
everything reachable only from them. One invisible entry point commonly
costs two or more findings.

This skill turns that off, correctly, in four steps. Steps 1–3 are the
work. Step 4 is the upstream offer, and has rules of its own.

## Step 0 — read the coverage table first

cgg ships a built-in rule table covering 340+ distinct frameworks across
36 languages, and **tells you which ones it recognised in your tree**.
Run it and read stderr before writing anything:

```bash
cgg ./src
```

Real output, from `cgg` on a Flask application:

```text
framework coverage
  recognised     flask (network, 130 entries) · click (cli, 31 entries)
                 celery (queue, 3 entries)
  seen, no rules py-context-manager — found in 2 file(s), entries NOT enumerated
                   detected, but no entry point matched its rules
                 pytest — found in 34 file(s), entries NOT enumerated
                   cgg has no entry rules for pytest; the runner -- not any
                   code in the tree -- collects and invokes everything.
                   [...]

  Entry-node coverage is PARTIAL. Handlers of the frameworks listed under
  "seen, no rules" are not represented and will still appear unreferenced.
```

Three outcomes, three different jobs:

- **Listed under `recognised`** — already handled. If a handler is still
  flagged, the cause is something else; check `--why-live` before
  writing a rule.
- **Listed under `seen, no rules`** — cgg knows the framework is there
  and cannot enumerate it. This is your case. The indented reason under
  each name is the rule's own `gap` prose, and it says whether a rule
  can fix it or whether the routes live somewhere cgg cannot read
  (Next.js file-system routing, Blazor `.razor` markup). The reason
  `detected, but no entry point matched its rules` means something
  different — see Step 3.
- **Not listed at all** — an in-house framework, or one cgg has never
  heard of. Also your case; you supply the `detect` marker too.

Two things about the table itself:

- It is printed only when at least one framework was **detected**. Pass
  `--framework-coverage` to print it unconditionally.
- Above six `seen, no rules` entries the list collapses to a count and
  six ids. `--framework-coverage` also restores the full list and each
  entry's reason.

## Step 1 — identify the shape, not the framework

What cgg needs depends on *how* control is handed off, not on which
framework does it. Find one flagged handler and look at its definition:

| | Shape | Looks like | Fix |
| --- | --- | --- | --- |
| **A** | marker on the definition | `@app.route(...)`, `@GetMapping`, `#[get("/")]`, `@shared_task` | `attributes` — **easiest** |
| **B** | callable passed as a value | `app.get("/x", handler)`, `RegisterWorkflow(W)` | `registrars` |
| **C** | inline closure at the call site | `app.get("/x", (req,res) => {...})` | `registrars` — same as B |
| **D** | base class / interface | `class X(nn.Module): def forward`, `implements Runnable`, `: IJob` | `base_types` + `methods` |
| **E** | a string names it | `'photos#index'`, `"App\C@method"` | `registrars` **plus `string_targets = true`** |
| **F** | separate file by path | `new Worker('./w.js')`, CUDA `__global__` | `registrars` (path) / `attributes` (pragma) |

**Bucket C usually needs nothing.** The handler body is lexically inside
a call cgg already reaches, so it is already live; a rule only adds the
*route name* to the graph. If a closure handler was reported dead
anyway, the cause is something else.

Confirm the diagnosis before fixing it:

```bash
cgg ./src --why-live 'MyHandler::method$'
```

With no `-o`, the proof prints to stdout. `NOT REACHED — no path from
any known root` confirms cgg has no path to it. If it prints a proof
path, the finding had a different cause.

## Step 2 — write a `[[framework]]` rule

Rules live in `cgg-deadcode.toml`. A `[[framework]]` block is the right
tool: it uses the same machinery as the built-in rules, so the handler
becomes live *and* gets an entry node in the graph, and the coverage
table stops listing the framework as a gap.

```toml
[[framework]]
id       = "myfw"        # appears in the node name and coverage table
language = "python"      # plugin id: python, javascript, typescript, java, ...
kind     = "network"     # see below — eight values

# DETECTION — the gate for registrars/base_types/methods. See the
# hazard note below: it is NOT a gate for `attributes`.
detect       = ["myfw"]              # import path prefixes
# detect_paths = ["config/routes.rb"] # or a path suffix convention

# MATCHERS — pick the ones for your shape. All optional, all combinable.
attributes = ["endpoint", "handler"]   # A: decorator/annotation keys
registrars = ["get", "post"]           # B/C/F: `app.get("/x", handler)`
base_types = ["BaseHandler"]           # D: class/interface that declares it
methods    = ["handle", "process"]     # D: which methods of those types

string_targets = false  # E: allow a string argument to NAME the handler
node = true             # false = mark a root but mint no node (see below)
```

Those are every key `[[framework]]` accepts. The file is parsed with
`deny_unknown_fields`, so a typo is a hard error rather than a silent
no-op. (The built-in table's `RuleSpec` has two extra fields —
`detect_calls` and `gap` — that a TOML rule cannot set.)

**Get `kind` right.** It is part of the entry node's name and therefore
of every `--filter '<framework-entry>::network::'` query someone runs to
enumerate attack surface. The eight values are `network`, `queue`,
`schedule`, `cli`, `ffi`, `lifecycle` (the default), `test` and
`public`. Use `network` only for things reachable from outside the
process. A worker that trusted internal code enqueues is `queue`; a
`forward`/`onCreate`/lifecycle hook is `lifecycle`; `public` is for a
callable the *language* exposes to anyone with no framework involved
(a Solidity `public`/`external` function).

**Shape E needs `string_targets = true`.** `registrars` alone matches
the registration call but will not decode a string argument into a
handler name. Verified: a Ruby rule with `registrars = ["route"]` and no
`string_targets` reports `detected, but no entry point matched its
rules`; adding `string_targets = true` mints
`<framework-entry>::network::myrouter::route('photos#index')`. It is
off by default because decoding strings everywhere turns any
`session.get("user_id")` into an entry point as soon as some callable
happens to be named `user_id`.

**Set `node = false` for shapes with no identity.** A rule matching
every `forward` in the repo would mint one node fanning out to every
model — visually useless. Root-marking is the right outcome there. Keep
`node = true` when the entry has a name worth reading: a route, a queue,
a scheduled job.

**Matchers compare case-insensitively, and against both the full
attribute key and its last segment.** `attributes = ["route"]` matches
`@app.route`, `@bp.route` and `@api.route`. `base_types` likewise
matches on the last segment with generic arguments stripped, so
`base_types = ["Module"]` matches `class Net(nn.Module)` — and also any
unrelated class named `Module`.

### The `detect` hazard, in both directions

`detect` is **not** uniformly a gate, and getting this wrong fails
silently in one of two opposite ways. Both behaviours are verified:

- **A rule whose only matchers are `registrars`, `base_types` or
  `methods`, with no `detect` and no `detect_paths`, never fires and is
  never even reported as a gap** — the coverage table prints
  `recognised (none)` / `seen, no rules (none)` and the rule is
  invisible. Always give those rules a `detect`.
- **An `attributes`-only rule with no `detect` fires everywhere.** When
  a rule declares no detection at all, cgg falls back to treating the
  marker itself as the evidence (this is how CUDA's `__global__` works).
  A rule with `attributes = ["endpoint"]` and no `detect` matched a file
  that imports nothing at all. That is usually not what you want:
  add `detect` so `endpoint` cannot mean "every `endpoint` in every
  codebase".

Notes that save a round trip:

- If the framework has no import (PHP's WordPress, an ambient global),
  use `detect_paths` with a path-suffix convention instead.
- `attributes` only works where cgg captures attributes — rust, python,
  java, csharp, javascript, typescript, php, kotlin and cpp. On other
  languages it matches nothing; use `registrars` or `base_types`.
- Verify the config was found. Discovery searches upward from each
  analyzed path and then the working directory, so
  `cgg /path/to/project` from anywhere picks it up — but an unmatched
  rule shows in the coverage table as "detected, but no entry point
  matched its rules".

### When `roots` is still the right tool

`[[framework]]` needs a detectable framework. For a one-off — a single
handler registered by something cgg cannot see at all — the older
mechanism still applies:

```toml
roots = ["^myapp::handlers::.*"]      # regex; `glob:` prefix for glob
root_attributes = ["glob:@endpoint*"]  # bucket A only
```

**Use `roots`, not `[[allow]]`.** `[[allow]]` suppresses a finding
*without* making it live, so the handler disappears from the report but
its private helpers keep getting flagged. That is the wrong tool here:
the handler genuinely *is* an entry point.

## Step 3 — verify it worked

Never assume. Count before and after:

```bash
cgg ./src --dead-code --dead-code-format json --dead-code-report /tmp/before.json
# edit cgg-deadcode.toml
cgg ./src --dead-code --dead-code-format json --dead-code-report /tmp/after.json

python3 - <<'EOF'
import json
b=json.load(open('/tmp/before.json'))
a=json.load(open('/tmp/after.json'))
print(f"{len(b['findings'])} -> {len(a['findings'])} findings")
print("stale patterns:", a['summary'].get('stale_suppressions') or "none")
EOF
```

(`--dead-code-report` is the explicit form. `-o graph.mmd` also works and
puts the report at `graph.mmd.deadcode.json` — the sidecar extension
follows `--dead-code-format`, so the default text format lands at
`.deadcode.txt`. `stale_suppressions` is omitted from the JSON when
empty, which is why the read above uses `.get`.)

Three checks:

- **The coverage table moved the framework out of "seen, no rules".**
  This is the fastest signal that the rule fired at all. If it now reads
  "detected, but no entry point matched its rules", `detect` worked and
  the matchers did not. If the framework vanishes from the table
  entirely, the rule never fired — see the `detect` hazard above.
- **The count dropped by more than one per handler.** If a handler had
  private helpers, they should have gone too. If only the handler
  disappeared, the rule is in `[[allow]]` rather than a `[[framework]]`
  block or `roots`.
- **`stale_suppressions` is empty.** Any `roots`/`[[allow]]` pattern
  matching nothing is listed there. A rule that silently matches nothing
  is worse than no rule, because it looks like it is working.

Worked example, verified end to end. An in-house decorator plus a
PyTorch-style model, all flagged before the rule:

```text
before:  ['forward', 'genuinely_unused', 'list_orders']
```

```toml
[[framework]]
id       = "acme"
language = "python"
kind     = "network"
detect   = ["acme"]
attributes = ["endpoint"]

[[framework]]
id       = "acme-ml"
language = "python"
kind     = "lifecycle"
detect     = ["acme.ml"]
base_types = ["AcmeModule"]
methods    = ["forward"]
node       = false      # no identity worth naming — root-mark only
```

```text
after:   ['genuinely_unused']

framework coverage
  recognised     acme (network, 1 entry) · acme-ml (lifecycle, 1 entry)
  seen, no rules (none)
```

The private helpers of both handlers (`_fmt`, `_project`) disappear too,
because a root propagates liveness transitively. That drop — 3 findings
to 1, with the one genuine finding surviving — is the signal the rule is
doing its job rather than just muting output.

## Step 4 — offer to send it upstream

**Only after the rule is verified working**, and **only if the framework
is public**, offer to contribute it back so every cgg user benefits.

### Decide whether it is public

Public means: available from a package registry (PyPI, npm, crates.io,
Maven Central, NuGet, Packagist, RubyGems, pkg.go.dev) **or** has a
publicly reachable source repository. Check the import marker:

```bash
grep -rhoE "(import|from|use|require|using)\s+\S*framework_name\S*" src/ | sort -u | head
```

If the import path is a public package name (`flask`, `@nestjs/common`,
`org.springframework.*`, `github.com/gin-gonic/gin`), it is public.

**Treat it as private if any of these hold** — do not offer, and say
why in one line:

- The import path is an internal namespace (`com.acme.internal.*`,
  `@acme/`, a private registry, a vendored path, a relative import).
- The repository is private, or you cannot tell.
- The framework name does not resolve to a public package.
- The user has said anything indicating the code is confidential.

When in doubt, it is private. The cost of not offering is a missed
contribution; the cost of offering wrongly is prompting someone to
publish details of proprietary internals.

### Ask, and show exactly what would be sent

Never send anything without explicit approval in the current
conversation. Prior approval for one framework does not carry to
another. Present the exact artifact first:

> This rule is for **Flask**, which is public (PyPI: `flask`). Want me
> to prepare an upstream PR to cgg so every user gets this built in?
>
> It would contain only this, and nothing from your codebase:
>
> ```toml
> [[framework]]
> id         = "flask"
> language   = "python"
> kind       = "network"
> detect     = ["flask"]
> attributes = ["route", "get", "post", "put", "delete"]
> ```
>
> No file paths, symbol names, or code from your repository are
> included.

### What may be shared, and what may not

| Share | Never share |
| --- | --- |
| The public framework's name and version | Your file paths or directory layout |
| Its public import marker | Your symbol or module names |
| The generic attribute/base-type keys | Any snippet of your source |
| The shape bucket (A–F) | Your `cgg-deadcode.toml` verbatim |
| A fixture written from the framework's **own public docs** | A fixture reduced from your code |

If a rule cannot be expressed without naming something of the user's,
it is not a generic framework rule — keep it local.

### Preparing the PR

Only on approval. Do not push to any remote the user has not named.

1. Write a minimal fixture from the framework's **published
   documentation** — not from their repository.
2. Add a `RuleSpec` to the built-in `SPECS` table in
   `crates/cgg-core/src/frameworks/rules.rs`, alongside the existing
   entries. `RuleSpec` is the Rust form of the same rule and carries two
   fields TOML does not: `detect_calls` and `gap`. The table's own tests
   enforce that every rule is either detectable or declares a `gap`
   (`every_rule_is_either_detectable_or_a_declared_gap`,
   `a_rule_with_no_matchers_must_declare_why`).
3. **Add an application to `APPS` in `scripts/benchmark.sh` that uses
   the framework** — one field of that entry names the rule id.
   `scripts/docs-check.py` fails on any enumerating rule with no app
   behind it, so skipping this blocks the commit. A rule that genuinely
   has no real-world application goes in `APPS_UNVERIFIED` instead, and
   the declaration is the point. Then check it actually fires with
   `scripts/framework-coverage.py`.
4. Add an integration test in `crates/cgg/tests/frameworks.rs` asserting
   the entry node appears with the right kind and the handler is not
   reported.
5. `cargo test --workspace` and `python3 scripts/docs-check.py` must
   pass.
6. Open the PR against the cgg repository, describing the framework, its
   shape, and the registry it is published on.

If the user declines, or the framework is private: keep the rule in
their `cgg-deadcode.toml`, commit it to *their* repo, and move on. A
local `[[framework]]` block runs through exactly the same engine as a
built-in one — upstreaming is a convenience for others, never a
requirement.

## What cannot be expressed yet

Be honest rather than writing a rule that silently does nothing:

- **The HTTP method is not a separate field.** The route string *is*
  captured — `@app.route("/users/<int:uid>")` becomes
  `<framework-entry>::network::flask::route('/users/<int:uid>')`, and
  `@app.post("/users")` becomes `post('/users')`. But the verb is only
  there when the decorator name carries it; a `methods=["GET"]` keyword
  is not decoded, and there is no structured `method` + `path` pair on
  the entry to query.
- **Base-type matching is by name, not by resolved type.** `base_types`
  is a real matcher, but it compares the declared base's last segment
  case-insensitively with generics stripped. `base_types = ["Module"]`
  catches `nn.Module` and every unrelated `Module` in the tree. Write
  the most specific segment you can. A rule with `methods` and no
  `base_types` is weaker still — the structural-typing escape hatch for
  Go interfaces and Elixir behaviours, matching on method name alone.
- **Attribute capture does not cover every language.** `attributes` and
  `root_attributes` work on the nine plugins listed in Step 2 and match
  nothing on the rest. Don't guess the list from memory — a run whose
  `--ignore-attributes` matched nothing prints the current one, and the
  dead-code report's capability table has an `attrs` column per
  language.
- **Framework config files are used for detection, never parsed.**
  `detect_paths` can prove a framework is present from a path like
  `config/routes.rb` or `routes/web.php`, but nothing reads the routes
  declared inside them, so routing that exists only there is out of
  reach. (Rails, Django and Laravel *are* recognised through their
  registrar calls and string handler forms — check the coverage table
  before writing a rule for them.)

## Anti-patterns

1. **Reaching for `[[allow]]` instead of `roots`.** Suppresses the
   handler but leaves its helpers flagged.
2. **Writing a broad pattern to make the report quiet.** `".*"` in
   `roots` marks the entire codebase live. The report is then clean and
   worthless.
3. **Shipping a rule without re-running cgg.** The two silent failures
   above — a matcher-only rule with no `detect`, and a shape-E rule
   without `string_targets` — both produce a config file that looks
   right and does nothing.
4. **Sending an upstream PR without asking**, or asking once and
   treating it as standing permission.
5. **Upstreaming a rule derived from private code.** Even if the
   framework is public, the *fixture* must come from its public docs.
