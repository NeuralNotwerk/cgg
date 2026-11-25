//! Framework rules and extraction for Perl, R, Julia and PowerShell: one
//! fixture per rule row, asserting the entry (or root mark) lands on the
//! callable the host calls and not on its same-named neighbours.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary")
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

/// Run `--dead-code` over `dir` and return (qualified names reported,
/// root qualified names with their labels, stderr).
fn dead_code(dir: &Path) -> (BTreeSet<String>, Vec<(String, String)>, String) {
    let out = cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-confidence",
            "low",
            "--jobs",
            "1",
        ])
        .arg(dir)
        .assert()
        .success();
    let output = out.get_output();
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json report");
    let findings = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap().to_string())
        .collect();
    let roots = doc["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["qualified_name"].as_str().unwrap_or_default().to_string(),
                r.to_string(),
            )
        })
        .collect();
    (
        findings,
        roots,
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// The framework entry nodes of a run: `<framework-entry>::…` names.
fn entries(dir: &Path) -> (BTreeSet<String>, String) {
    let out = cgg()
        .args(["-t", "json", "--jobs", "1"])
        .arg(dir)
        .assert()
        .success();
    let output = out.get_output();
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json graph");
    let callables = &doc["callables"];
    let list: Vec<&serde_json::Value> = match callables {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(o) => o.values().collect(),
        _ => Vec::new(),
    };
    let names = list
        .iter()
        .filter_map(|c| c["qualified_name"].as_str())
        .filter(|q| q.starts_with("<framework-entry>"))
        .map(str::to_string)
        .collect();
    (names, String::from_utf8_lossy(&output.stderr).to_string())
}

// ---------------------------------------------------------------------
// Perl
// ---------------------------------------------------------------------

#[test]
fn perl_builtin_lifecycle_subs_are_roots_and_compare_exactly() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/Tied.pm",
        "package Tied;\n\
         sub TIEHASH { bless {}, shift }\n\
         sub FETCH { 1 }\n\
         sub DESTROY { }\n\
         sub AUTOLOAD { }\n\
         sub destroy { }\n\
         1;\n",
    );
    let (dead, _, _) = dead_code(tmp.path());
    for live in [
        "Tied::TIEHASH",
        "Tied::FETCH",
        "Tied::DESTROY",
        "Tied::AUTOLOAD",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("Tied::destroy"), "{dead:?}");
}

#[test]
fn moose_lifecycle_roots_build_hooks_case_sensitively() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/Point.pm",
        "package Point;\nuse Moose;\n\
         has size => (is => 'ro', builder => '_build_size');\n\
         sub BUILD { }\n\
         sub BUILDARGS { }\n\
         sub DEMOLISH { }\n\
         sub _build_size { 3 }\n\
         sub build { }\n\
         1;\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("moose-lifecycle"), "{err}");
    for live in [
        "Point::BUILD",
        "Point::BUILDARGS",
        "Point::DEMOLISH",
        "Point::_build_size",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("Point::build"), "{dead:?}");
}

#[test]
fn irssi_commands_and_signals_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "scripts/away.pl",
        "use Irssi;\n\
         sub cmd_away { }\n\
         sub sig_msg { }\n\
         sub unused { }\n\
         Irssi::command_bind('away', \\&cmd_away);\n\
         Irssi::signal_add('message public', 'sig_msg');\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("irssi"), "{err}");
    assert!(
        names
            .iter()
            .any(|n| n.contains("irssi") && n.contains("away")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.contains("irssi") && n.contains("message public")),
        "{names:?}"
    );
    let (dead, _, _) = dead_code(tmp.path());
    assert!(
        !dead.contains("cmd_away") && !dead.contains("sig_msg"),
        "{dead:?}"
    );
    assert!(dead.contains("unused"), "{dead:?}");
}

#[test]
fn minion_tasks_are_entries_through_a_plugin_load() {
    let tmp = TempDir::new().unwrap();
    // No `use Minion`: the app loads it as a Mojolicious plugin.
    write(
        tmp.path(),
        "lib/App.pm",
        "package App;\nuse Mojo::Base 'Mojolicious';\n\
         sub _resize { }\n\
         sub startup {\n  my $self = shift;\n  $self->plugin(Minion => {Pg => 'x'});\n\
         \x20 $self->minion->add_task(resize => \\&_resize);\n\
         \x20 $self->minion->add_task(cleanup => sub { my $job = shift; });\n}\n1;\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("minion"), "{err}");
    assert!(
        names
            .iter()
            .any(|n| n.contains("minion") && n.contains("resize")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.contains("minion") && n.contains("cleanup")),
        "the bareword-named inline task is an entry too: {names:?}"
    );
}

// ---------------------------------------------------------------------
// PowerShell
// ---------------------------------------------------------------------

#[test]
fn chocolatey_au_hooks_are_roots_only_in_update_scripts() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "automatic/7zip/update.ps1",
        "Import-Module Chocolatey-AU\n\
         function global:au_SearchReplace { @{} }\n\
         function global:au_GetLatest { @{ Version = '1' } }\n\
         function Get-Unused { 'x' }\n\
         update -ChecksumFor none\n",
    );
    write(
        tmp.path(),
        "scripts/helpers.ps1",
        "function global:au_GetLatest { 'not an update script' }\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("chocolatey-au"), "{err}");
    let live = |q: &str| !dead.iter().any(|d| d.ends_with(q));
    assert!(live("au_SearchReplace"), "{dead:?}");
    assert!(dead.iter().any(|d| d.contains("Get-Unused")), "{dead:?}");
    // Two `au_GetLatest`s: only the update script's is rooted.
    assert_eq!(
        dead.iter().filter(|d| d.ends_with("au_GetLatest")).count(),
        1,
        "{dead:?}"
    );
}

// ---------------------------------------------------------------------
// R
// ---------------------------------------------------------------------

#[test]
fn targets_is_disclosed_and_tar_source_includes_the_r_directory() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "_targets.R",
        "library(targets)\ntar_source()\n\
         run_all <- function() fit_model(1)\n\
         list(tar_target(model, run_all()))\n",
    );
    write(
        tmp.path(),
        "R/functions.R",
        "fit_model <- function(x) x\nunused_helper <- function() 1\n",
    );
    let out = cgg()
        .args(["-t", "json", "--jobs", "1"])
        .arg(tmp.path())
        .assert()
        .success();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("targets"), "{err}");
    let doc: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).unwrap();
    // `run_all` binds `fit_model` through the include `tar_source()`
    // makes: certain, where a by-name guess across files is `medium`.
    let edges = doc["edges"].as_array().unwrap();
    assert!(edges.iter().any(|e| e["confidence"] == "high"), "{edges:?}");
}

#[test]
fn rmd_and_qmd_chunks_are_analysed() {
    let tmp = TempDir::new().unwrap();
    let doc = "---\ntitle: r\n---\n\nProse with f(x) in it.\n\n```{r setup}\nlocal_fn <- function(x) summarise_data(x)\n```\n\nMore prose.\n\n```{r}\nlocal_fn(1)\n```\n";
    write(tmp.path(), "report.Rmd", doc);
    write(
        tmp.path(),
        "talk.qmd",
        &doc.replace("local_fn", "quarto_fn"),
    );
    write(tmp.path(), "R/f.R", "summarise_data <- function(x) x\n");
    let out = cgg()
        .args(["-t", "json", "--jobs", "1"])
        .arg(tmp.path())
        .assert()
        .success();
    let doc: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).unwrap();
    let text = doc.to_string();
    assert!(text.contains("\"local_fn\""), "the .Rmd chunk's function");
    assert!(text.contains("\"quarto_fn\""), "the .qmd chunk's function");
}

// ---------------------------------------------------------------------
// Julia
// ---------------------------------------------------------------------

#[test]
fn julia_functions_passed_as_values_are_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/model.jl",
        "using DifferentialEquations\n\
         function lorenz!(du, u, p, t)\n  du[1] = u[1]\nend\n\
         condition(u, t, i) = u[1] > 1\n\
         function never_used() end\n\
         function main()\n  prob = ODEProblem(lorenz!, [1.0], (0.0, 1.0))\n\
         \x20 solve(prob; callback = DiscreteCallback(condition, x -> x))\nend\nmain()\n",
    );
    let (dead, _, _) = dead_code(tmp.path());
    assert!(!dead.contains("lorenz!"), "{dead:?}");
    assert!(!dead.contains("condition"), "{dead:?}");
    assert!(dead.contains("never_used"), "{dead:?}");
}

#[test]
fn julia_route_handlers_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Router.jl",
        "using HTTP\n\
         serve_asset(req) = HTTP.Response(200)\n\
         function make_router()\n  r = HTTP.Router()\n\
         \x20 HTTP.register!(r, \"GET\", \"/asset\", serve_asset)\n  r\nend\n",
    );
    write(
        tmp.path(),
        "app/routes.jl",
        "using Genie\nfunction index() \"hi\" end\nroute(\"/\", index)\n\
         route(\"/w\") do\n  \"w\"\nend\n",
    );
    write(
        tmp.path(),
        "api/server.jl",
        "using Oxygen\n@get \"/greet\" function(req)\n  \"hello\"\nend\n\
         ping(req) = \"pong\"\n@post(\"/ping\", ping)\n",
    );
    let (names, err) = entries(tmp.path());
    for (rule, route) in [
        ("http-jl", "GET /asset"),
        ("genie", "/w"),
        ("oxygen", "/greet"),
        ("oxygen", "/ping"),
    ] {
        assert!(
            names.iter().any(|n| n.contains(rule) && n.contains(route)),
            "{rule} {route}: {names:?}\n{err}"
        );
    }
    let (dead, _, _) = dead_code(tmp.path());
    for live in ["serve_asset", "index", "ping"] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
}

#[test]
fn franklin_utils_functions_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "utils.jl",
        "using Dates\nhfun_bar(vname) = string(vname)\nfunction lx_baz(com, _) \"x\" end\nfunction helper() end\n",
    );
    write(tmp.path(), "src/other.jl", "hfun_x() = 1\n");
    let (names, err) = entries(tmp.path());
    assert!(err.contains("franklin"), "{err}");
    assert!(
        names.iter().any(|n| n.ends_with("franklin::hfun_bar")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("franklin::lx_baz")),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.ends_with("hfun_x")), "{names:?}");
}

#[test]
fn julia_methods_of_external_generics_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Pkg.jl",
        "module MyPkg\nusing Tables\nimport Logging\n\
         function Tables.columns(t::Int) end\n\
         Logging.shouldlog(l::Int, args...) = true\n\
         function Base.length(t::Int) 1 end\n\
         module Inner\nfunction g() end\nend\n\
         function Inner.h() end\n\
         function Unknown.k() end\n\
         function plain() end\nend\n",
    );
    let (dead, _, _) = dead_code(tmp.path());
    for live in [
        "MyPkg.Tables.columns",
        "MyPkg.Logging.shouldlog",
        "MyPkg.Base.length",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    // A module of the tree, or one nothing imports, is no external generic.
    assert!(dead.contains("MyPkg.Inner.h"), "{dead:?}");
    assert!(dead.contains("MyPkg.Unknown.k"), "{dead:?}");
    assert!(dead.contains("MyPkg.plain"), "{dead:?}");
}
