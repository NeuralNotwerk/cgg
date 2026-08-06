//! Framework entry points — trust kinds, rules, and coverage disclosure.
//!
//! cgg resolves calls it can see in source. Frameworks invoke user code
//! by means that are not calls: a decorator registers a route, a base
//! class declares a contract the runtime calls, a path string names a
//! worker module. Control enters the application there, and no call
//! expression exists for a resolver to bind.
//!
//! An **entry node** is the mirror image of the exit nodes
//! `synthesize_exit_nodes` already mints. Where an exit node is a *sink*
//! for an unresolved call, an entry node is a *source* for an
//! unresolvable caller. It makes the graph literally true rather than
//! annotated-true: today a route handler has in-degree zero, which is a
//! claim ("nothing calls this") and a false one.
//!
//! Two commitments run through this module, both inherited from the
//! dead-code report:
//!
//! **An entry node is an inference, not an observation.** An exit node
//! is minted from a call site cgg *saw* and could not resolve. An entry
//! node asserts a caller that appears nowhere in the tree. So the
//! evidence bar is higher and the labelling is deliberately redundant —
//! see [`FRAMEWORK_ENTRY_DISCLAIMER`].
//!
//! **Partial coverage is disclosed, never implied to be complete.** A
//! bare list of twelve routes invites the reader to believe that is all
//! of them. The same list beside "django — imports found, entries NOT
//! enumerated" tells them exactly where to look manually. That is what
//! [`FrameworkCoverage`] is for, and why it names three things
//! separately rather than one.

pub mod rules;

use serde::{Deserialize, Serialize};

use crate::ids::{CallableId, FileId};

/// Sentinel file path that entry nodes belong to, mirroring
/// `<external>` and `<stdlib>`.
pub const FRAMEWORK_ENTRY_SENTINEL: &str = "<framework-entry>";

/// The mandatory preamble wherever entry nodes are reported.
///
/// Held as a constant in `cgg-core` — rather than composed by whichever
/// formatter happens to run — so that no output path can omit it.
pub const FRAMEWORK_ENTRY_DISCLAIMER: &str = "\
BEST EFFORT — ENTRY NODES ARE INFERRED, NOT OBSERVED. cgg synthesizes \
these from framework markers it recognises: a decorator, a base type, a \
registration call. Nothing in your source states that the call happens. \
Coverage is partial — a framework cgg does not recognise contributes no \
entry nodes at all, and its handlers will still appear unreferenced. The \
coverage table states exactly which frameworks were recognised and which \
imports were seen but not understood. Absence of an entry node is not \
evidence that no entry exists.";

/// The limit that matters most for security work, stated wherever the
/// `network` kind is surfaced.
pub const REACHABILITY_NOT_TAINT: &str = "\
cgg shows call REACHABILITY, not data flow. \"Reachable from a network \
entry node\" means control can get there. It does not mean \
attacker-controlled data does: there is no taint tracking, no sanitizer \
awareness, no branch-condition analysis, and no notion of which \
parameter carries the payload. Use it to bound where to look, never to \
conclude something is exploitable.";

/// What kind of trust boundary control crosses at an entry point.
///
/// Framework entry is **not** the same as attack surface. A single
/// `<framework-entry>` bucket would mix `POST /api/users` with
/// `Encoder.forward`, and those are not remotely the same thing. The
/// kind is part of the entry node's qualified name so it is filterable:
///
/// ```text
/// cgg ./src --filter '<framework-entry>::network::' -n 3
/// cgg ./src --exclude-partial '<framework-entry>::lifecycle::'
/// ```
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustKind {
    /// HTTP route, gRPC service, websocket, GraphQL resolver. The only
    /// kind that is unambiguously attack surface.
    Network,
    /// Celery task, BullMQ consumer, Sidekiq job, Kafka listener.
    /// Untrusted depending on who can enqueue.
    Queue,
    /// `@Scheduled`, cron, timer. No external input.
    Schedule,
    /// `@click.command`, argv entry. A local trust boundary.
    Cli,
    /// `#[no_mangle]`, pyo3/napi/JNI export. Depends on the host.
    Ffi,
    /// `forward`, `onCreate`, `Drop`, `ServeHTTP` on an internal type.
    /// The conservative default: no trust boundary is asserted.
    #[default]
    Lifecycle,
    /// Test harness entry.
    Test,
}

impl TrustKind {
    /// Stable, greppable slug. This is the segment that appears in an
    /// entry node's qualified name, so it is part of the CLI contract.
    pub fn slug(self) -> &'static str {
        match self {
            TrustKind::Network => "network",
            TrustKind::Queue => "queue",
            TrustKind::Schedule => "schedule",
            TrustKind::Cli => "cli",
            TrustKind::Ffi => "ffi",
            TrustKind::Lifecycle => "lifecycle",
            TrustKind::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "network" => TrustKind::Network,
            "queue" => TrustKind::Queue,
            "schedule" => TrustKind::Schedule,
            "cli" => TrustKind::Cli,
            "ffi" => TrustKind::Ffi,
            "lifecycle" => TrustKind::Lifecycle,
            "test" => TrustKind::Test,
            _ => return None,
        })
    }

    /// Whether this boundary carries input cgg considers untrusted by
    /// default. `Queue` is deliberately excluded: it depends entirely on
    /// who can enqueue, which cgg cannot see.
    pub fn untrusted_input(self) -> bool {
        matches!(self, TrustKind::Network)
    }
}

/// How control is handed off — §3's six shapes. Recorded on each entry
/// so a reader can tell a decorator match from a base-type guess.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryShape {
    /// **A** — a marker on the definition: `@app.route`, `@GetMapping`,
    /// `#[get("/")]`.
    Attribute,
    /// **B** — the callable passed as a value: `app.get("/x", handler)`.
    Registrar,
    /// **C** — an inline closure at the registration site. Already
    /// reachable; the node exists to name the *route*.
    Closure,
    /// **D** — a base class or interface contract: `nn.Module.forward`,
    /// `IJob.Execute`.
    BaseType,
    /// **E** — a string names the target: `'photos#index'`,
    /// `"App\C@method"`.
    StringTarget,
    /// **F** — a separate unit named by path or pragma:
    /// `new Worker('./w.js')`, `__global__`.
    ModulePath,
}

impl EntryShape {
    pub fn slug(self) -> &'static str {
        match self {
            EntryShape::Attribute => "attribute",
            EntryShape::Registrar => "registrar",
            EntryShape::Closure => "closure",
            EntryShape::BaseType => "base-type",
            EntryShape::StringTarget => "string-target",
            EntryShape::ModulePath => "module-path",
        }
    }
}

/// One framework's recognition rules.
///
/// A single flat struct rather than a per-shape enum, because it is also
/// the user-authored TOML shape: a `[[framework]]` block in
/// `cgg-deadcode.toml` deserializes straight into this. Every matcher
/// list is independently optional, so one rule can cover a framework
/// that uses several shapes at once (Actix: attributes *and* a
/// registrar).
///
/// `detect` is what stops a user's own `Worker` class from matching. A
/// rule contributes nothing until an import in `detect` (or a path in
/// `detect_paths`) has been seen somewhere in that language.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkRule {
    /// Framework id — the `flask` in
    /// `<framework-entry>::network::flask::route("/users")`.
    pub id: String,
    /// Plugin id this rule applies to (`python`, `java`, …).
    pub language: String,
    /// Trust boundary the entry crosses.
    #[serde(default)]
    pub kind: TrustKind,

    /// Import path prefixes proving the framework is in use
    /// (`flask`, `org.springframework.web.bind.annotation`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detect: Vec<String>,
    /// Path suffixes that prove it instead, for frameworks discovered by
    /// file convention (`config/routes.rb`, `routes/web.php`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detect_paths: Vec<String>,

    /// **Shape A** — attribute / decorator keys, compared after
    /// `attribute_key` normalization (`app.route`, `GetMapping`, `get`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<String>,
    /// **Shape B/C** — registrar calls that take a handler:
    /// `app.get`, `Route::get`, `router.HandleFunc`. Matched on the
    /// method name, with an optional `receiver.` prefix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registrars: Vec<String>,
    /// **Shape D** — base classes / interfaces whose methods the runtime
    /// invokes (`nn.Module`, `Runnable`, `IJob`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_types: Vec<String>,
    /// Which methods of `base_types` are entry points. Empty means
    /// every method of a matching type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,

    /// Whether a string argument may *name* the handler — shape E.
    ///
    /// Off by default, and deliberately so. Decoding strings is only
    /// correct for frameworks that actually route that way (Rails,
    /// Laravel, WordPress). Applied everywhere it turns any
    /// `session.get("user_id")` into an entry point as soon as some
    /// project callable happens to be called `user_id`, which is how a
    /// crates.io run produced two "routes" and neither was one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub string_targets: bool,

    /// Whether a matching entry mints a node, or only marks a root.
    ///
    /// §8: one `torch:Module.__call__` node fanning out to every model
    /// in the repo is visually useless. Entry nodes earn their place
    /// where the entry has *identity* — a route, a queue, a command.
    #[serde(default = "default_true")]
    pub node: bool,
}

fn default_true() -> bool {
    true
}

impl FrameworkRule {
    /// Whether this rule can produce entries at all, as opposed to only
    /// identifying that the framework is present. A rule with no
    /// matchers lands in [`FrameworkCoverage::seen_no_rules`].
    pub fn has_matchers(&self) -> bool {
        !self.attributes.is_empty()
            || !self.registrars.is_empty()
            || !self.base_types.is_empty()
    }
}

/// One synthesized entry point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameworkEntry {
    /// Framework id (`flask`).
    pub framework: String,
    pub kind: TrustKind,
    pub shape: EntryShape,
    /// The entry's identity — a route, a queue name, a command, a
    /// method name. Empty when the marker carried none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
    /// The user callable control enters.
    pub target: CallableId,
    pub target_name: String,
    /// The marker that fired, verbatim (`@app.route("/users")`).
    pub evidence: String,
    pub file: FileId,
    pub site_line: u32,
    /// Whether this entry mints a node, or only marks a root.
    pub node: bool,
}

impl FrameworkEntry {
    /// Qualified name of the entry node standing in for this entry:
    /// `<framework-entry>::network::flask::route("/users")`.
    ///
    /// The kind comes before the framework so `--filter
    /// '<framework-entry>::network::'` selects the whole attack surface
    /// across every framework at once.
    pub fn node_name(&self) -> String {
        let tail = if self.route.is_empty() {
            self.target_name
                .rsplit(|c| c == ':' || c == '.' || c == '/')
                .next()
                .unwrap_or(&self.target_name)
                .to_string()
        } else {
            self.route.clone()
        };
        format!(
            "{FRAMEWORK_ENTRY_SENTINEL}::{}::{}::{}",
            self.kind.slug(),
            self.framework,
            tail
        )
    }
}

/// A framework whose entries cgg enumerated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognisedFramework {
    pub id: String,
    pub language: String,
    pub kind: TrustKind,
    pub entries: u32,
}

/// A framework cgg identified but has no entry rules for. Naming this is
/// what makes partial coverage usable rather than misleading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenFramework {
    pub id: String,
    pub language: String,
    /// How many files carried its import marker.
    pub files: u32,
    /// Why no entries came out — always user-facing prose.
    pub reason: String,
}

/// A language in which cgg has no framework rules at all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncoveredLanguage {
    pub language: String,
    pub files: u32,
}

/// The three-part coverage disclosure.
///
/// The failure mode to avoid is a partial list that *reads* as complete.
/// A SecEng enumerating attack surface on a Rails app must not conclude
/// "3 network entries" when the true answer is 300 and cgg simply cannot
/// parse `routes.rb`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameworkCoverage {
    /// Copied in by the engine so no formatter can drop it.
    pub disclaimer: String,
    /// Frameworks whose entries were enumerated.
    pub recognised: Vec<RecognisedFramework>,
    /// Frameworks seen but not understood.
    pub seen_no_rules: Vec<SeenFramework>,
    /// Languages with no framework rules whatsoever.
    pub no_markers: Vec<UncoveredLanguage>,
    /// Total entry nodes minted (entries with `node: true`).
    pub nodes_minted: u32,
    /// Entries that only marked a root — bucket D, per §8.
    pub root_marks_only: u32,
}

impl FrameworkCoverage {
    pub fn new() -> Self {
        Self {
            disclaimer: FRAMEWORK_ENTRY_DISCLAIMER.to_string(),
            ..Default::default()
        }
    }

    /// Whether anything at all was recognised. Used to decide whether
    /// the coverage block is worth printing.
    pub fn is_empty(&self) -> bool {
        self.recognised.is_empty()
            && self.seen_no_rules.is_empty()
            && self.no_markers.is_empty()
    }

    /// Count of `network`-kind entries — the security-relevant number,
    /// and the one that must never be read as complete.
    pub fn network_entries(&self) -> u32 {
        self.recognised
            .iter()
            .filter(|r| r.kind == TrustKind::Network)
            .map(|r| r.entries)
            .sum()
    }

    /// Render the §2 disclosure block.
    ///
    /// Lives here rather than in `cgg-format` so that every surface —
    /// stderr summary, audit log, dead-code report — prints the same
    /// three sections in the same order, and none of them can quietly
    /// drop the "seen, no rules" list that makes the rest honest.
    pub fn render_text(&self) -> String {
        let mut s = String::new();
        s.push_str("framework coverage\n");

        if self.recognised.is_empty() {
            s.push_str("  recognised     (none)\n");
        } else {
            let items: Vec<String> = self
                .recognised
                .iter()
                .map(|r| {
                    let plural = if r.entries == 1 { "entry" } else { "entries" };
                    format!("{} ({}, {} {plural})", r.id, r.kind.slug(), r.entries)
                })
                .collect();
            s.push_str(&wrap_field("  recognised    ", &items.join(" · ")));
        }

        if self.seen_no_rules.is_empty() {
            s.push_str("  seen, no rules (none)\n");
        } else {
            for (i, f) in self.seen_no_rules.iter().enumerate() {
                let label = if i == 0 { "  seen, no rules" } else { "                " };
                s.push_str(&format!(
                    "{label} {} — found in {} file(s), entries NOT enumerated\n",
                    f.id, f.files
                ));
                // The reason is what makes the gap actionable: it tells
                // the reader whether to look at `routes.rb` by hand or
                // to write a local rule. A bare name would not.
                if !f.reason.is_empty() {
                    s.push_str(&format!("                   ({})\n", f.reason));
                }
            }
        }

        if !self.no_markers.is_empty() {
            let total: u32 = self.no_markers.iter().map(|l| l.files).sum();
            let langs: Vec<&str> = self.no_markers.iter().map(|l| l.language.as_str()).collect();
            s.push_str(&format!(
                "  no rules      {total} file(s) in languages with no framework rules ({})\n",
                langs.join(", ")
            ));
        }

        s.push('\n');
        s.push_str(
            "  Entry-node coverage is PARTIAL. Handlers of the frameworks listed under\n  \
             \"seen, no rules\" are not represented and will still appear unreferenced.\n",
        );
        if self.network_entries() > 0 {
            s.push_str(
                "  Reachability from a `network` entry is not proof of attacker-controlled\n  \
                 data flow — cgg does no taint tracking.\n",
            );
        }
        s
    }
}

/// Wrap a long single-line field under a fixed label column.
fn wrap_field(label: &str, body: &str) -> String {
    const WIDTH: usize = 76;
    let indent = " ".repeat(label.len() + 1);
    let mut out = String::from(label);
    let mut col = label.len();
    let mut first = true;
    for part in body.split(" · ") {
        let piece_len = part.len() + 3;
        if !first && col + piece_len > WIDTH {
            out.push('\n');
            out.push_str(&indent);
            col = indent.len();
        } else if !first {
            out.push_str(" · ");
            col += 3;
        } else {
            out.push(' ');
            col += 1;
        }
        out.push_str(part);
        col += part.len();
        first = false;
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclaimer_states_the_inference() {
        assert!(FRAMEWORK_ENTRY_DISCLAIMER.contains("BEST EFFORT"));
        assert!(FRAMEWORK_ENTRY_DISCLAIMER.contains("INFERRED, NOT OBSERVED"));
        // The single most important sentence: absence is not evidence.
        assert!(FRAMEWORK_ENTRY_DISCLAIMER.contains("Absence of an entry node is not"));
    }

    #[test]
    fn trust_kind_slugs_round_trip() {
        for k in [
            TrustKind::Network,
            TrustKind::Queue,
            TrustKind::Schedule,
            TrustKind::Cli,
            TrustKind::Ffi,
            TrustKind::Lifecycle,
            TrustKind::Test,
        ] {
            assert_eq!(TrustKind::parse(k.slug()), Some(k));
        }
        assert_eq!(TrustKind::parse("nonsense"), None);
    }

    #[test]
    fn only_network_is_asserted_untrusted() {
        assert!(TrustKind::Network.untrusted_input());
        // Queue depends entirely on who can enqueue, which cgg cannot
        // see — asserting it would be a guess dressed as a finding.
        assert!(!TrustKind::Queue.untrusted_input());
        assert!(!TrustKind::Lifecycle.untrusted_input());
    }

    fn entry(route: &str, kind: TrustKind) -> FrameworkEntry {
        FrameworkEntry {
            framework: "flask".into(),
            kind,
            shape: EntryShape::Attribute,
            route: route.into(),
            target: CallableId::new(0),
            target_name: "svc.list_users".into(),
            evidence: "@app.route(\"/users\")".into(),
            file: FileId::new(0),
            site_line: 3,
            node: true,
        }
    }

    #[test]
    fn node_name_puts_kind_before_framework_so_filters_cut_across() {
        let e = entry("route(\"/users\")", TrustKind::Network);
        assert_eq!(
            e.node_name(),
            "<framework-entry>::network::flask::route(\"/users\")"
        );
        // The documented attack-surface query must select it.
        assert!(e.node_name().starts_with("<framework-entry>::network::"));
    }

    #[test]
    fn routeless_entry_falls_back_to_the_target_name() {
        let e = entry("", TrustKind::Queue);
        assert_eq!(e.node_name(), "<framework-entry>::queue::flask::list_users");
    }

    #[test]
    fn coverage_names_what_it_could_not_do() {
        // The honesty test: a report with zero recognised entries must
        // still say which frameworks it saw and skipped.
        let mut c = FrameworkCoverage::new();
        c.seen_no_rules.push(SeenFramework {
            id: "rails".into(),
            language: "ruby".into(),
            files: 7,
            reason: "config/routes.rb is not parsed".into(),
        });
        let text = c.render_text();
        assert!(text.contains("rails"), "{text}");
        assert!(text.contains("entries NOT enumerated"), "{text}");
        assert!(text.contains("PARTIAL"), "{text}");
        assert!(!text.contains("no callables"));
    }

    #[test]
    fn taint_caveat_only_appears_when_a_network_entry_does() {
        let mut c = FrameworkCoverage::new();
        assert!(!c.render_text().contains("taint"));
        c.recognised.push(RecognisedFramework {
            id: "flask".into(),
            language: "python".into(),
            kind: TrustKind::Network,
            entries: 12,
        });
        assert_eq!(c.network_entries(), 12);
        assert!(c.render_text().contains("taint"));
    }

    #[test]
    fn a_rule_without_matchers_cannot_produce_entries() {
        let mut r = FrameworkRule {
            id: "django".into(),
            language: "python".into(),
            detect: vec!["django".into()],
            ..Default::default()
        };
        assert!(!r.has_matchers());
        r.attributes.push("login_required".into());
        assert!(r.has_matchers());
    }
}
