//! Framework entry-node integration tests.
//!
//! One fixture per shape × trust kind from §3 of the design, asserting
//! that the entry node appears with the right kind and that the handler
//! is no longer flagged unreferenced.
//!
//! The single most important test in the file is
//! [`coverage_names_a_framework_it_cannot_enumerate`]. Everything else
//! checks that recognised frameworks produce the right nodes; that one
//! checks that an *un*recognised framework is named rather than silently
//! counted as zero — which is the difference between a partial map and a
//! misleading one.

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

/// Run cgg over `dir` and return (mermaid graph, stderr).
fn run(dir: &Path, extra: &[&str]) -> (String, String) {
    let out = dir.join("g.mmd");
    let mut cmd = cgg();
    cmd.args(["-o"]).arg(&out).args(extra).arg(dir);
    let assert = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

// ---------------------------------------------------------------------
// Shape A — a marker on the definition
// ---------------------------------------------------------------------

#[test]
fn shape_a_python_flask_route_is_a_network_entry() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\n\
         @app.route(\"/users\")\ndef list_users():\n    return _render()\n\n\
         def _render():\n    return \"ok\"\n",
    );
    let (g, err) = run(tmp.path(), &[]);
    assert!(
        g.contains("network::flask::route"),
        "expected a flask network entry node:\n{g}"
    );
    assert!(g.contains("-->|entry|"), "expected an entry edge:\n{g}");
    // Three independent signals, per §1: the sentinel prefix, the node
    // tag, and the edge label — plus a header that survives copy-paste.
    assert!(g.contains("⟨framework entry callback⟩"), "{g}");
    assert!(g.contains("%% cgg:"), "{g}");
    assert!(g.contains("SYNTHESIZED"), "{g}");
    assert!(err.contains("flask (network, 1 entry)"), "{err}");
}

#[test]
fn shape_a_java_spring_mapping_keeps_its_route() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "UserController.java",
        "package com.example;\n\
         import org.springframework.web.bind.annotation.GetMapping;\n\
         public class UserController {\n\
         \x20 @GetMapping(\"/users\")\n\
         \x20 public String listUsers() { return render(); }\n\
         \x20 private String render() { return \"ok\"; }\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    // `attribute_key` discards arguments by design; the route survives
    // only because a second accessor keeps them (open decision 1).
    assert!(
        g.contains("network::spring::GetMapping('/users')"),
        "the route must reach the node name:\n{g}"
    );
}

#[test]
fn shape_a_csharp_attribute_is_captured() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Api.cs",
        "using Microsoft.AspNetCore.Mvc;\n\
         public class UsersController : ControllerBase {\n\
         \x20 [HttpGet(\"/users\")]\n\
         \x20 public string List() { return Render(); }\n\
         \x20 private string Render() { return \"ok\"; }\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("network::aspnet-mvc::"), "{g}");
}

#[test]
fn shape_a_php_symfony_attribute_is_captured() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "BlogController.php",
        "<?php\nnamespace App\\Controller;\n\
         use Symfony\\Component\\Routing\\Attribute\\Route;\n\n\
         class BlogController {\n\
         \x20   #[Route('/blog', name: 'blog_list')]\n\
         \x20   public function list() { return $this->fetch(); }\n\
         \x20   private function fetch() { return []; }\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("network::symfony::Route('/blog')"), "{g}");
}

// ---------------------------------------------------------------------
// Shape B — the callable passed as a value
// ---------------------------------------------------------------------

#[test]
fn shape_b_express_named_handler_binds_to_its_route() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.js",
        "const express = require('express');\nconst app = express();\n\
         function listUsers(req, res) { res.send(render()); }\n\
         function render() { return 'ok'; }\n\
         app.get('/users', listUsers);\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("network::express::get('/users')"), "{g}");
}

#[test]
fn shape_b_go_router_verb_matches_case_insensitively() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.go",
        "package main\n\nimport \"net/http\"\n\n\
         func handleUsers(w http.ResponseWriter, r *http.Request) { render(w) }\n\
         func render(w http.ResponseWriter) {}\n\
         func main() { http.HandleFunc(\"/users\", handleUsers) }\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("network::net-http::"), "{g}");
}

// ---------------------------------------------------------------------
// Shape C — an inline closure at the registration site
// ---------------------------------------------------------------------

#[test]
fn shape_c_anonymous_handler_still_gets_its_route_named() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.js",
        "const express = require('express');\nconst app = express();\n\
         app.post('/admin/users', (req, res) => { res.send(audit()); });\n\
         function audit() { return 'a'; }\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    // The closure body was always reachable. The point of the node is
    // that `POST /admin/users` is the fact a reader wants, and
    // `handler_at_3` is not.
    assert!(g.contains("network::express::post('/admin/users')"), "{g}");
}

// 0.14.0: an MCP tool is a `tool` entry, named by what the model calls it
// (its `name`, else the function), never by its description.
#[test]
fn rmcp_tools_are_tool_entries_named_by_the_tool() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/server.rs",
        "use rmcp::{tool, tool_router};\n\
         pub struct Server;\n\
         #[tool_router]\n\
         impl Server {\n\
         \x20   #[tool(description = \"Scan the given ports\")]\n\
         \x20   fn scan_ports(&self) -> u32 { probe() }\n\
         \x20   #[tool(name = \"http_get\", description = \"Fetch a URL\")]\n\
         \x20   fn fetch(&self) -> u32 { probe() }\n\
         }\n\
         fn probe() -> u32 { 1 }\n",
    );
    let (g, err) = run(tmp.path(), &[]);
    assert!(g.contains("tool::rmcp::scan_ports"), "{g}");
    assert!(g.contains("tool::rmcp::http_get"), "{g}");
    assert!(!g.contains("Scan the given ports"), "{g}");
    assert!(!g.contains("network::rmcp"), "{g}");
    assert!(err.contains("rmcp (tool, 2 entries)"), "{err}");
    assert!(err.contains("prompt injection"), "{err}");
}

// 0.14.0 registrar capture limits: a five-argument registration
// (`server.tool(name, description, schema, annotations, handler)`), a
// registrar called through a file-level alias (`const f =
// useCopilotAction as any`), and a user rule's dotted registrar all hand
// their handler to the rule.
#[test]
fn registrar_capture_reaches_long_calls_aliases_and_dotted_user_verbs() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "cgg-deadcode.toml",
        "[[framework]]\nid = \"mytools\"\nlanguage = \"typescript\"\nkind = \"tool\"\n\
         detect = [\"mytools\"]\nregistrars = [\"server.tool\", \"useAction\"]\n",
    );
    write(
        tmp.path(),
        "src/server.ts",
        "import { Server, useAction } from 'mytools';\n\
         const server = new Server();\n\
         const useActionTyped = useAction as any;\n\
         server.tool('get_weather', 'Weather now', {}, { readOnly: true }, async () => probe());\n\
         useActionTyped({ name: 'open_page', handler: async () => probe() });\n\
         function probe() { return 1; }\n",
    );
    let toml = tmp.path().join("cgg-deadcode.toml");
    let (g, err) = run(tmp.path(), &["--roots", toml.to_str().unwrap()]);
    assert!(g.contains("tool::mytools::get_weather"), "{g}\n{err}");
    assert!(g.contains("tool::mytools::open_page"), "{g}\n{err}");
}

#[test]
fn shape_c_go_inline_martini_route_is_an_entry() {
    // Martini's README writes every route as an inline `func_literal`,
    // which had no node, so the rule could enumerate nothing.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.go",
        "package main\n\nimport \"github.com/go-martini/martini\"\n\n\
         func main() {\n\tm := martini.Classic()\n\
         \tm.Get(\"/hello\", func() string { return greet() })\n\tm.Run()\n}\n\
         func greet() string { return \"hi\" }\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("network::martini::get('/hello')"), "{g}");
}

#[test]
fn electron_ipc_handlers_are_entries_and_other_on_calls_are_not() {
    // `ipcMain.handle` / `ipcRenderer.on` are Electron's entry points,
    // reached through a qualified receiver even when it is written
    // `window.electron.ipcRenderer`. A bare `$(x).on('click')` in the
    // same app is not IPC and must not be claimed.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.js",
        "const { app, ipcMain } = require('electron');\n\
         ipcMain.handle('fonts:list', async () => listFonts());\n\
         app.on('ready', () => boot());\n\
         function listFonts() { return []; }\nfunction boot() {}\n",
    );
    write(
        tmp.path(),
        "renderer.js",
        "const { shell } = require('electron');\n\
         window.electron.ipcRenderer.on('mt::saved', () => refresh());\n\
         $('#x').on('click', () => refresh());\n\
         function refresh() {}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("electron::handle('fonts:list')"), "{g}");
    assert!(g.contains("electron::on('ready')"), "{g}");
    assert!(g.contains("electron::on('mt::saved')"), "{g}");
    assert!(!g.contains("electron::on('click')"), "{g}");
}

#[test]
fn ordinary_callbacks_do_not_mint_synthesized_handlers() {
    // The gate that keeps a test suite from minting a node per block:
    // `describe('...', () => {})` has exactly the shape of a route
    // registration and is not one.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "spec.js",
        "describe('thing', () => { it('works', () => { check(); }); });\n\
         function check() {}\n\
         const doubled = [1,2].map(x => x * 2);\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(!g.contains("handler_at_"), "{g}");
}

// ---------------------------------------------------------------------
// Shape D — a base class or interface declares the contract
// ---------------------------------------------------------------------

#[test]
fn shape_d_torch_module_marks_a_root_without_minting_a_node() {
    // §8: an entry node per bucket-D method is visually useless. The
    // payoff is in the dead-code report, not in the graph.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "model.py",
        "import torch.nn as nn\n\nclass Encoder(nn.Module):\n\
         \x20   def forward(self, x):\n        return self._project(x)\n\n\
         \x20   def _project(self, x):\n        return x\n",
    );
    let (g, err) = run(tmp.path(), &[]);
    assert!(err.contains("torch (lifecycle"), "{err}");
    assert!(
        !g.contains("<framework-entry>") && !g.contains("&lt;framework-entry&gt;"),
        "a lifecycle base type must not mint a node:\n{g}"
    );
    assert!(err.contains("1 root-marked only"), "{err}");
}

#[test]
fn shape_d_kivy_app_build_marks_a_root_without_minting_a_node() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        "from kivy.app import App\n\nclass MakeraApp(App):\n\
         \x20   def build(self):\n        return self._root()\n\n\
         \x20   def _root(self):\n        return None\n",
    );
    let (g, err) = run(tmp.path(), &[]);
    assert!(err.contains("kivy (lifecycle"), "{err}");
    assert!(
        !g.contains("<framework-entry>") && !g.contains("&lt;framework-entry&gt;"),
        "a lifecycle base type must not mint a node:\n{g}"
    );
    assert!(err.contains("root-marked only"), "{err}");
}

#[test]
fn shape_d_quartz_ijob_does_mint_a_node_because_the_entry_has_identity() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Job.cs",
        "using Quartz;\npublic class CleanupJob : IJob {\n\
         \x20 public void Execute() { Purge(); }\n\
         \x20 private void Purge() {}\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("schedule::quartz::"), "{g}");
}

// ---------------------------------------------------------------------
// Shape E — a string names the target
// ---------------------------------------------------------------------

#[test]
fn shape_e_rails_string_routing_reaches_the_controller_action() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config/routes.rb",
        "require 'rails'\nget 'photos', to: 'photos#index'\n",
    );
    write(
        tmp.path(),
        "app/controllers/photos_controller.rb",
        "class PhotosController < ApplicationController\n\
         \x20 def index\n    render_list\n  end\n\
         \x20 def render_list; end\nend\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    // Rails handlers are named methods never referenced as callables
    // anywhere — string routing is the only link that exists.
    assert!(g.contains("network::rails::get('photos')"), "{g}");
    assert!(g.contains("PhotosController::index"), "{g}");
}

#[test]
fn shape_e_laravel_supports_both_the_string_and_the_array_form() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/Http/Controllers/UserController.php",
        "<?php\nnamespace App\\Http\\Controllers;\n\
         use Illuminate\\Routing\\Controller;\n\n\
         class UserController extends Controller {\n\
         \x20   public function index() { return $this->render(); }\n\
         \x20   private function render() { return 'ok'; }\n}\n",
    );
    write(
        tmp.path(),
        "routes/web.php",
        "<?php\nuse Illuminate\\Support\\Facades\\Route;\n\
         use App\\Http\\Controllers\\UserController;\n\n\
         Route::get('/users', [UserController::class, 'index']);\n\
         Route::post('/legacy', 'UserController@index');\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    // <= L7 string form and >= L8 array form must both land.
    assert!(g.contains("network::laravel::get('/users')"), "{g}");
    assert!(g.contains("network::laravel::post('/legacy')"), "{g}");
}

// ---------------------------------------------------------------------
// Shape F — a separate unit named by path or pragma
// ---------------------------------------------------------------------

#[test]
fn shape_f_worker_module_path_rescues_a_whole_file() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "queue.js",
        "const { Worker } = require('worker_threads');\n\
         function enqueue() { new Worker('./jobs/resize.js'); }\n\
         module.exports = { enqueue };\n",
    );
    write(
        tmp.path(),
        "jobs/resize.js",
        "function resize(buf) { return normalize(buf); }\n\
         function normalize(buf) { return buf; }\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("queue::worker-threads::Worker('./jobs/resize.js')"),
        "{g}"
    );
    // The module surface, not every callable in it: `normalize` is a
    // private helper, and pointing an entry at it would misdescribe the
    // module.
    let entry_edges = g.lines().filter(|l| l.contains("-->|entry|")).count();
    assert_eq!(entry_edges, 1, "expected one entry edge, got:\n{g}");
}

#[test]
fn shape_f_cuda_kernel_is_an_entry_despite_the_unparsable_launch() {
    // §8: `tree-sitter-cpp` reads `saxpy<<<a,b>>>(x)` as nested
    // comparisons, so the launch produces no edge at all. The qualifier
    // is the evidence instead — no grammar fight.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "k.cpp",
        "__global__ void saxpy(int n, float a) { helper(n); }\n\
         __device__ void helper(int n) {}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(g.contains("lifecycle::cuda::saxpy"), "{g}");
}

// ---------------------------------------------------------------------
// Coverage honesty — the most important test in the set
// ---------------------------------------------------------------------

#[test]
fn coverage_names_a_framework_it_cannot_enumerate() {
    // A SecEng enumerating attack surface must not read "0 entries" as
    // "no attack surface". A framework cgg sees and cannot read has to
    // be NAMED.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "page.js",
        "import Head from 'next/head';\nexport default function Page() { return null; }\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    assert!(err.contains("seen, no rules"), "{err}");
    assert!(err.contains("nextjs"), "{err}");
    assert!(err.contains("entries NOT enumerated"), "{err}");
    assert!(err.contains("PARTIAL"), "{err}");
}

#[test]
fn coverage_reports_a_recognised_framework_that_matched_nothing_as_a_gap() {
    // "flask (network, 0 entries)" reads as "this app has no routes".
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "util.py",
        "from flask import Flask\n\ndef helper():\n    return 1\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    assert!(!err.contains("0 entries"), "{err}");
    assert!(err.contains("no entry point matched"), "{err}");
}

#[test]
fn coverage_discloses_languages_with_no_rules_at_all() {
    // Uses a language that genuinely has NO rule in the table. Fortran
    // used to serve here and no longer can; if this ever fails because
    // the named language gained a rule, that is the fix — pick another
    // from the "no rules" line of `--framework-coverage`, do not weaken
    // the assertion.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "m.v", "module top(input clk);\nendmodule\n");
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route(\"/x\")\ndef x():\n    return 1\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    assert!(err.contains("no framework rules"), "{err}");
    assert!(err.contains("verilog"), "{err}");
}

#[test]
fn the_taint_caveat_rides_with_every_network_entry() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route(\"/x\")\ndef x():\n    return 1\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    assert!(err.contains("taint"), "{err}");
    assert!(err.contains("not proof of attacker-controlled"), "{err}");
}

// ---------------------------------------------------------------------
// Gating, opt-out, and the dead-code payoff
// ---------------------------------------------------------------------

#[test]
fn an_undetected_framework_contributes_nothing() {
    // Without the import gate, every decorator named `route` in every
    // codebase becomes attack surface.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "import os\n\n@route(\"/users\")\ndef list_users():\n    return 1\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(!g.contains("framework-entry"), "{g}");
}

#[test]
fn no_entry_nodes_restores_the_previous_default_graph() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route(\"/users\")\ndef list_users():\n    return 1\n",
    );
    let (with, _) = run(tmp.path(), &[]);
    assert!(with.contains("framework-entry"));
    let (without, err) = run(tmp.path(), &["--no-entry-nodes"]);
    assert!(!without.contains("framework-entry"), "{without}");
    assert!(
        !without.contains("%% cgg: &lt;framework-entry&gt;"),
        "{without}"
    );
    assert!(!err.contains("framework coverage"), "{err}");
}

// `--no-test-entries` drops the harness node and its one-arrow-per-case
// fan, and nothing else: a framework entry in the same tree stays, and the
// dead-code report is unchanged.
#[test]
fn no_test_entries_drops_only_the_harness_nodes() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route(\"/users\")\ndef list_users():\n    return 1\n",
    );
    write(
        tmp.path(),
        "lib.rs",
        "pub fn add(a: u32) -> u32 { a + 1 }\n\n\
         #[tokio::test]\nasync fn adds() { assert_eq!(add(1), 2); }\n",
    );
    let (with, _) = run(tmp.path(), &[]);
    assert!(with.contains("::test::cargo-test"), "{with}");
    let (without, _) = run(tmp.path(), &["--no-test-entries"]);
    assert!(!without.contains("::test::"), "{without}");
    assert!(without.contains("/users"), "{without}");
    // The case is not orphaned in the dead-code sense: same report
    // with the harness node and without it.
    let (_, dead_with) = run(tmp.path(), &["--dead-code"]);
    let (_, dead_without) = run(tmp.path(), &["--dead-code", "--no-test-entries"]);
    let findings = |e: &str| -> Vec<String> {
        e.lines()
            .filter(|l| l.contains("adds"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(findings(&dead_with), findings(&dead_without));
}

#[test]
fn entry_nodes_remove_the_bucket_d_dead_code_cascade() {
    // The design's §5 measurement: a framework-invoked method and its
    // private helper are both reported, in both languages. One invisible
    // entry point costs two findings.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "model.py",
        "import torch.nn as nn\n\nclass Encoder(nn.Module):\n\
         \x20   def forward(self, x):\n        return self._project(x)\n\n\
         \x20   def _project(self, x):\n        return x\n",
    );
    write(
        tmp.path(),
        "main.go",
        "package main\n\nimport \"net/http\"\n\ntype Handler struct{}\n\n\
         func (h Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) { h.render(w) }\n\
         func (h Handler) render(w http.ResponseWriter) {}\n\nfunc main() {}\n",
    );

    let out = tmp.path().join("d.mmd");
    let assert = cgg()
        .args(["--dead-code", "--dead-code-confidence", "medium", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let err = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        err.contains("0 callable(s) marked unreferenced"),
        "framework-invoked methods and their helpers must not be \
         reported:\n{err}"
    );

    // And with the mechanism off, the findings come back — which is what
    // makes the claim above meaningful rather than a tautology.
    let out2 = tmp.path().join("d2.mmd");
    let assert2 = cgg()
        .args([
            "--no-entry-nodes",
            "--dead-code",
            "--dead-code-confidence",
            "medium",
            "-o",
        ])
        .arg(&out2)
        .arg(tmp.path())
        .assert()
        .success();
    let err2 = String::from_utf8_lossy(&assert2.get_output().stderr).to_string();
    assert!(
        !err2.contains("0 callable(s) marked unreferenced"),
        "expected findings without entry nodes:\n{err2}"
    );
}

#[test]
fn a_user_rule_covers_a_framework_cgg_does_not_ship() {
    // The coverage table's gap list is only actionable if a user can act
    // on it without waiting for a release.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "cgg-deadcode.toml",
        "[[framework]]\nid = \"myfw\"\nlanguage = \"python\"\nkind = \"network\"\n\
         detect = [\"myfw\"]\nattributes = [\"endpoint\"]\n",
    );
    write(
        tmp.path(),
        "svc.py",
        "import myfw\n\n@myfw.endpoint(\"/z\")\ndef handle():\n    return 1\n",
    );
    // No `--roots`: discovery starts from the analyzed path, so a
    // project's rules apply wherever the run was launched from.
    let (g, err) = run(tmp.path(), &[]);
    assert!(g.contains("network::myfw::endpoint('/z')"), "{g}\n{err}");
}

#[test]
fn a_misspelled_key_in_a_user_rule_is_an_error() {
    // REGRESSION: `atributes` was silently ignored, so the rule matched
    // nothing and the run looked healthy.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "cgg-deadcode.toml",
        "[[framework]]\nid = \"myfw\"\nlanguage = \"python\"\nkind = \"network\"\n\
         detect = [\"myfw\"]\natributes = [\"endpoint\"]\n",
    );
    write(tmp.path(), "svc.py", "def handle():\n    return 1\n");
    let out = cgg().arg("--dead-code").arg(tmp.path()).assert().failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("atributes"), "{err}");
}

#[test]
fn the_attack_surface_query_from_the_docs_actually_works() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\nimport torch.nn as nn\napp = Flask(__name__)\n\n\
         @app.route(\"/users\")\ndef list_users():\n    return _render()\n\n\
         def _render():\n    return \"ok\"\n\n\
         class Encoder(nn.Module):\n    def forward(self, x):\n        return x\n",
    );
    let out = tmp.path().join("q.mmd");
    cgg()
        .args(["--filter", "<framework-entry>::network::", "-n", "3", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g = fs::read_to_string(&out).unwrap();
    assert!(
        g.contains("list_users"),
        "the handler must be selected:\n{g}"
    );
    assert!(g.contains("_render"), "and its blast radius:\n{g}");
    assert!(
        !g.contains("Encoder"),
        "lifecycle entries are not attack surface:\n{g}"
    );
}

// ---------------------------------------------------------------------
// Trust boundaries that are not frameworks at all
// ---------------------------------------------------------------------

#[test]
fn solidity_visibility_is_the_trust_boundary() {
    // Solidity has no framework to detect: `public`/`external` mean any
    // address on the chain can call the function, and that is the whole
    // attack surface of a contract.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Vault.sol",
        "contract Vault {\n\
         \x20   function deposit() public payable { _credit(); }\n\
         \x20   function _credit() internal { }\n\
         \x20   function drain() external { }\n\
         \x20   function peek() private view returns (uint) { return 1; }\n\
         }\n",
    );
    let (graph, err) = run(tmp.path(), &["--framework-coverage"]);
    assert!(err.contains("solidity-public"), "{err}");
    assert!(
        graph.contains("::public::solidity-public::deposit"),
        "{graph}"
    );
    assert!(
        graph.contains("::public::solidity-public::drain"),
        "{graph}"
    );
    // The whole point of reading visibility is that these are excluded
    // as *entries*. `_credit` still appears as an ordinary callable —
    // `deposit` really does call it — so the assertion has to be about
    // the entry-node prefix, not about the name appearing at all.
    assert!(
        !graph.contains("::public::solidity-public::_credit"),
        "internal is not attack surface: {graph}"
    );
    assert!(
        !graph.contains("::public::solidity-public::peek"),
        "{graph}"
    );
}

#[test]
fn rust_ffi_exports_are_entries_from_outside_the_tree() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    write(
        tmp.path(),
        "src/lib.rs",
        "#[no_mangle]\npub extern \"C\" fn c_entry() -> i32 { inner() }\n\
         fn inner() -> i32 { 1 }\n",
    );
    let (graph, _) = run(tmp.path(), &["--framework-coverage"]);
    assert!(graph.contains("::ffi::ffi-export::"), "{graph}");
    assert!(graph.contains("c_entry"), "{graph}");
}

// ---------------------------------------------------------------------
// A registrar handed a *type* rather than a callable
// ---------------------------------------------------------------------

#[test]
fn django_as_view_binds_to_the_classes_http_methods() {
    // Django's dominant modern idiom. The value reference resolves to
    // `as_view`, which is not a handler and binds to nothing — the entry
    // is every method of the class the rule calls an entry point.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "urls.py",
        "from django.urls import path\nfrom . import views\n\n\
         urlpatterns = [path('detail/', views.SiteView.as_view(), name='d')]\n",
    );
    write(
        tmp.path(),
        "views.py",
        "class SiteView:\n    def get(self, request):\n        return 1\n",
    );
    let (graph, _) = run(tmp.path(), &["--framework-coverage"]);
    assert!(
        graph.contains("::network::django::path('detail/')"),
        "{graph}"
    );
}

#[test]
fn drf_router_register_binds_the_viewsets_actions() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "urls.py",
        "from django.urls import path\n\
         from rest_framework.routers import DefaultRouter\nfrom . import views\n\n\
         router = DefaultRouter()\nrouter.register(r'sites', views.SiteViewSet)\n",
    );
    write(
        tmp.path(),
        "views.py",
        "class SiteViewSet:\n    def list(self, request):\n        return 1\n\
         \x20   def create(self, request):\n        return 2\n",
    );
    let (graph, _) = run(tmp.path(), &["--framework-coverage"]);
    assert!(
        graph.contains("::network::django::register('sites')"),
        "{graph}"
    );
}

// ---------------------------------------------------------------------
// Descriptor → implementation, across languages
// ---------------------------------------------------------------------

#[test]
fn a_proto_rpc_links_to_its_go_implementation() {
    // Neither file references the other: the .proto names no Go symbol,
    // and the Go file's link to the service runs through generated code
    // that is usually not committed.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.proto",
        "syntax = \"proto3\";\nservice Greeter {\n  rpc SayHello (Req) returns (Resp);\n}\n\
         message Req { string name = 1; }\nmessage Resp { string msg = 1; }\n",
    );
    write(tmp.path(), "go.mod", "module demo\ngo 1.21\n");
    write(
        tmp.path(),
        "server.go",
        "package main\n\ntype GreeterServer struct{}\n\n\
         func (s *GreeterServer) SayHello(r *Req) *Resp { return nil }\n",
    );
    let (graph, _) = run(tmp.path(), &[]);
    assert!(
        graph.contains("desc"),
        "descriptor edge must be tagged: {graph}"
    );
    assert!(graph.contains("SayHello"), "{graph}");
}

#[test]
fn a_bare_method_name_does_not_link_a_descriptor() {
    // `Get` appears in every codebase. Without the owner naming the
    // service this would manufacture an edge into unrelated code.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "svc.proto",
        "syntax = \"proto3\";\nservice Greeter {\n  rpc Get (Req) returns (Resp);\n}\n\
         message Req { string a = 1; }\nmessage Resp { string b = 1; }\n",
    );
    write(tmp.path(), "go.mod", "module demo\ngo 1.21\n");
    write(
        tmp.path(),
        "cache.go",
        "package main\n\ntype Store struct{}\n\nfunc (s *Store) Get() int { return 1 }\n",
    );
    let (graph, _) = run(tmp.path(), &[]);
    assert!(
        !graph.contains("|desc|"),
        "no owner match, no edge: {graph}"
    );
}

// ---------------------------------------------------------------------
// AWS Lambda — six languages, and the one framework whose entry point is
// conventionally named *outside* the source tree
// ---------------------------------------------------------------------
//
// Lambda is the case the framework feature exists for. Nothing in a
// handler's own file calls it, so before these rules every Lambda
// codebase reported its entire handler module as dead. Each runtime
// names the handler differently, which is why this is six rules rather
// than one.

#[test]
fn aws_lambda_go_start_binds_the_handler_value() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "go.mod", "module demo\ngo 1.21\n");
    write(
        tmp.path(),
        "main.go",
        "package main\n\nimport (\n\t\"context\"\n\t\
         \"github.com/aws/aws-lambda-go/lambda\"\n)\n\n\
         func work(s string) string { return s }\n\n\
         func HandleRequest(ctx context.Context, e string) (string, error) {\n\
         \treturn work(e), nil\n}\n\n\
         func main() { lambda.Start(HandleRequest) }\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("aws-lambda-go::main.HandleRequest"),
        "lambda.Start's argument is the entry point:\n{g}"
    );
}

#[test]
fn aws_lambda_python_handler_convention_and_powertools_routes() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        "from aws_lambda_powertools.event_handler import APIGatewayRestResolver\n\
         app = APIGatewayRestResolver()\n\n\
         def load(uid):\n    return uid\n\n\
         @app.get(\"/users\")\ndef get_user(uid):\n    return load(uid)\n\n\
         def lambda_handler(event, context):\n    return app.resolve(event, context)\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    // Shape A: the resolver route. (Mermaid HTML-escapes `<`/`>`, so the
    // fixture uses a path parameter-free route to keep the assertion
    // about the rule rather than about escaping.)
    assert!(
        g.contains("aws-lambda-powertools::get('/users')"),
        "powertools route should be an entry:\n{g}"
    );
    // The handler itself, by convention rather than by any marker.
    assert!(
        g.contains("aws-lambda::app.lambda_handler"),
        "lambda_handler is the conventional entry point:\n{g}"
    );
}

#[test]
fn a_lambda_handler_outside_a_lambda_project_is_not_an_entry() {
    // The detection gate is what makes a name convention safe. Without
    // an aws-lambda import this is just a function with a common name.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        "import json\n\ndef helper(e):\n    return e\n\n\
         def lambda_handler(event, context):\n    return helper(event)\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        !g.contains("framework-entry"),
        "no aws import, no claim:\n{g}"
    );
}

#[test]
fn aws_lambda_java_request_handler_is_a_declared_contract() {
    // The generic parameters matter: `RequestHandler<String, String>`
    // was truncated by the base-type splitter, so Java — the runtime
    // where the entry point is an actual interface — produced nothing.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Handler.java",
        "package com.example;\n\
         import com.amazonaws.services.lambda.runtime.Context;\n\
         import com.amazonaws.services.lambda.runtime.RequestHandler;\n\n\
         public class Handler implements RequestHandler<String, String> {\n\
         \tprivate String norm(String s) { return s.trim(); }\n\
         \tpublic String handleRequest(String in, Context ctx) { return norm(in); }\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("aws-lambda::com.example.Handler.handleRequest"),
        "handleRequest on a RequestHandler impl is the entry:\n{g}"
    );
}

#[test]
fn aws_cdk_binds_a_handler_string_to_a_function_in_another_file() {
    // The point of the CDK rule. The handler has no AWS import of its
    // own and nothing calls it — the only thing naming it is a string
    // in the infrastructure code.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/app.py",
        "def validate(e):\n    return True\n\n\
         def lambda_handler(event, context):\n    return validate(event)\n",
    );
    write(
        tmp.path(),
        "infra/stack.py",
        "from aws_cdk import Stack, aws_lambda as _lambda\n\
         from constructs import Construct\n\n\
         class ApiStack(Stack):\n\
         \tdef __init__(self, scope, cid):\n\
         \t\t_lambda.Function(self, \"Api\",\n\
         \t\t\thandler=\"app.lambda_handler\",\n\
         \t\t\tcode=_lambda.Code.from_asset(\"src\"))\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("aws-cdk::function('Api')"),
        "the CDK construct should mint an entry:\n{g}"
    );
    // And it must point at the real handler, not merely exist.
    assert!(
        g.contains("app.lambda_handler"),
        "the entry must bind the function the string names:\n{g}"
    );
}

#[test]
fn aws_cdk_typescript_binds_a_handler_from_an_options_object() {
    // TypeScript CDK puts the handler in an options object rather than
    // a keyword argument, and TS callables are not module-qualified —
    // so this resolves through the file-stem index instead.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/orders.ts",
        "function validate(e: any) { return !!e; }\n\
         export const processOrder = async (event: any) => validate(event);\n",
    );
    write(
        tmp.path(),
        "infra/stack.ts",
        "import * as cdk from \"aws-cdk-lib\";\n\
         import * as lambda from \"aws-cdk-lib/aws-lambda\";\n\n\
         export class S extends cdk.Stack {\n\
         \tconstructor(scope: any, id: string) {\n\
         \t\tsuper(scope, id);\n\
         \t\tnew lambda.Function(this, \"Orders\", {\n\
         \t\t\thandler: \"orders.processOrder\",\n\
         \t\t\tcode: lambda.Code.fromAsset(\"src\"),\n\t\t});\n\t}\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("aws-cdk::function('Orders')") && g.contains("processOrder"),
        "TS CDK options-object handler should bind:\n{g}"
    );
}

// ---------------------------------------------------------------------
// The other clouds
// ---------------------------------------------------------------------
//
// Same problem as Lambda — nothing in a handler's own file calls it —
// solved by each platform differently. Before 0.6.8 Google Cloud
// Functions had no rule in any language, and Azure, Cloudflare and Deno
// detected their framework while enumerating nothing from it.

#[test]
fn gcp_functions_binds_every_runtime_shape() {
    // One library across six runtimes, three registration mechanisms.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.py",
        "import functions_framework\n\ndef _r(x):\n    return x\n\n\
         @functions_framework.http\ndef hello_http(request):\n    return _r(request)\n",
    );
    write(
        tmp.path(),
        "index.js",
        "const functions = require('@google-cloud/functions-framework');\n\
         function build(x) { return x; }\n\
         functions.http('helloJs', (req, res) => build(req));\n",
    );
    write(
        tmp.path(),
        "Fn.java",
        "package com.example;\nimport com.google.cloud.functions.HttpFunction;\n\
         public class Fn implements HttpFunction {\n  private String r(String s){return s;}\n\
         \x20 public void service(Object req, Object res) { r(\"ok\"); }\n}\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("gcp-functions::main.hello_http"),
        "python decorator:\n{g}"
    );
    assert!(
        g.contains("gcp-functions::http('helloJs')"),
        "js registrar:\n{g}"
    );
    assert!(
        g.contains("gcp-functions::com.example.Fn.service"),
        "java contract:\n{g}"
    );
}

#[test]
fn azure_functions_binds_the_attribute_and_the_options_object() {
    // The v4 JavaScript model puts the handler in an options object,
    // which argument-position scanning missed entirely.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Fn.cs",
        "using Microsoft.Azure.Functions.Worker;\npublic class F {\n  private string R(string s) => s;\n\
         \x20 [Function(\"HttpTrigger\")]\n  public string Run(object req) => R(\"ok\");\n}\n",
    );
    write(
        tmp.path(),
        "index.js",
        "const { app } = require('@azure/functions');\nfunction build(x){return x;}\n\
         app.http('httpExample', { handler: async (req, ctx) => build(req) });\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("azure-functions::Function('HttpTrigger')"),
        "C# attribute:\n{g}"
    );
    assert!(
        g.contains("azure-functions::http('httpExample')"),
        "v4 options object:\n{g}"
    );
}

#[test]
fn cloudflare_worker_module_handlers_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "worker.ts",
        "import type { ExecutionContext } from '@cloudflare/workers-types';\n\
         function route(p: string) { return p; }\n\
         export default {\n  async fetch(request: Request): Promise<Response> {\n\
         \x20   return new Response(route('/'));\n  },\n\
         \x20 async scheduled(event: unknown) { route('cron'); },\n};\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("cloudflare-workers::fetch"),
        "module-worker fetch:\n{g}"
    );
    assert!(
        g.contains("cloudflare-workers::scheduled"),
        "cron handler:\n{g}"
    );
}

#[test]
fn firebase_python_triggers_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.py",
        "from firebase_functions import https_fn\n\ndef _b(x):\n    return x\n\n\
         @https_fn.on_request()\ndef api(req):\n    return _b(req)\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("firebase-functions::main.api"),
        "decorated trigger:\n{g}"
    );
}

/// An inline handler with no route string still binds.
///
/// The registration-shape gate required a leading string literal until
/// 0.6.8, which cost three platforms outright: `Deno.serve((req) => …)`,
/// Firebase's `onRequest((req, res) => …)` and Express middleware
/// `app.use(fn)` all carry a handler and no route. The string was never
/// what made the gate safe — the caller's verb gate is, and `describe`,
/// `it`, `map` and `then` are registrar verbs in no rule. Measured on
/// the corpus, removing it added 1.4% more nodes on Ghost and cost no
/// wall clock.
#[test]
fn a_closure_with_no_route_string_still_registers() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.ts",
        "import { serveDir } from \"jsr:@std/http/file-server\";\n\
         function build(p: string) { return p; }\n\
         Deno.serve((req: Request) => new Response(build(req.url)));\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("deno-http::handler_at_"),
        "an inline handler is still a registration:\n{g}"
    );
}

/// Cloudflare's legacy service-worker form binds when the worker is
/// detectable at all.
#[test]
fn cloudflare_legacy_addeventlistener_binds() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "index.js",
        "import { WorkerEntrypoint } from 'cloudflare:workers';\n\
         async function handleRequest(request) { return new Response('hi'); }\n\
         addEventListener('fetch', event => { event.respondWith(handleRequest(event.request)); });\n",
    );
    let (g, _) = run(tmp.path(), &[]);
    assert!(
        g.contains("cloudflare-workers::addeventlistener('fetch')"),
        "the event name is the identity:\n{g}"
    );
}

// ---------------------------------------------------------------------
// Every (platform, language) pair that claims to enumerate
// ---------------------------------------------------------------------
//
// Table-driven because the matrix is the point. cgg's claim is
// *equivalency* — that a Ruby Lambda is as well served as a Python one
// — and a claim like that is only worth what its worst-covered cell is.
// Before this table 13 of 45 pairs had a test and the rest had been
// checked by hand once, which is not the same thing.
//
// Each row is the smallest source that carries the platform's real
// registration shape, plus the substring the entry node must contain.
struct CloudCase {
    name: &'static str,
    file: &'static str,
    src: &'static str,
    expect: &'static str,
}

const CLOUD_CASES: &[CloudCase] = &[
    // ---- AWS Lambda ----
    CloudCase {
        name: "aws-lambda/ruby",
        file: "handler.rb",
        src: "require 'aws-sdk-s3'\ndef norm(s)\n  s\nend\ndef lambda_handler(event:, context:)\n  norm(event)\nend\n",
        expect: "aws-lambda::lambda_handler",
    },
    CloudCase {
        name: "aws-lambda/php",
        file: "handler.php",
        src: "<?php\nuse Bref\\Event\\Handler;\nclass MyHandler implements Handler {\n  private function r($s) { return $s; }\n  public function handle($e, $c) { return $this->r($e); }\n}\n",
        expect: "aws-lambda::MyHandler::handle",
    },
    CloudCase {
        name: "aws-lambda/kotlin",
        file: "H.kt",
        src: "package com.example\nimport com.amazonaws.services.lambda.runtime.RequestHandler\nclass H : RequestHandler<String, String> {\n  private fun n(s: String) = s\n  override fun handleRequest(i: String, c: Any): String = n(i)\n}\n",
        expect: "aws-lambda::com.example.H.handleRequest",
    },
    CloudCase {
        name: "aws-lambda/scala",
        file: "H.scala",
        src: "package com.example\nimport com.amazonaws.services.lambda.runtime.RequestHandler\nclass H extends RequestHandler[String, String] {\n  private def n(s: String): String = s\n  def handleRequest(i: String, c: Any): String = n(i)\n}\n",
        expect: "aws-lambda::",
    },
    CloudCase {
        name: "aws-lambda/groovy",
        file: "H.groovy",
        src: "package com.example\nimport com.amazonaws.services.lambda.runtime.RequestHandler\nclass H implements RequestHandler<String, String> {\n  private String n(String s) { s }\n  String handleRequest(String i, Object c) { n(i) }\n}\n",
        expect: "aws-lambda::",
    },
    CloudCase {
        name: "aws-lambda/swift",
        file: "H.swift",
        src: "import AWSLambdaRuntime\nstruct H: LambdaHandler {\n  func r(_ s: String) -> String { return s }\n  func handle(_ e: String, context: LambdaContext) async throws -> String { return r(e) }\n}\n",
        expect: "aws-lambda::",
    },
    CloudCase {
        name: "aws-lambda/cpp",
        file: "main.cpp",
        src: "#include <aws/lambda-runtime/runtime.h>\nusing namespace aws::lambda_runtime;\ninvocation_response r(invocation_request const& q);\ninvocation_response my_handler(invocation_request const& q) { return r(q); }\nint main() { run_handler(my_handler); return 0; }\n",
        expect: "aws-lambda::my_handler",
    },
    CloudCase {
        name: "aws-lambda/javascript",
        file: "index.js",
        src: "const { Logger } = require('@aws-lambda-powertools/logger');\nfunction v(e) { return !!e; }\nexports.handler = async (event) => v(event);\n",
        expect: "aws-lambda::handler",
    },
    CloudCase {
        name: "aws-lambda/csharp",
        file: "F.cs",
        src: "using Amazon.Lambda.Core;\npublic class F {\n  private string R(string s) => s;\n  public string FunctionHandler(string i, ILambdaContext c) => R(i);\n}\n",
        expect: "aws-lambda::F.FunctionHandler",
    },
    CloudCase {
        name: "lambda-runtime/rust",
        file: "main.rs",
        src: "use lambda_runtime::{service_fn, LambdaEvent, Error};\nfn p(v: &str) -> usize { v.len() }\nasync fn handler(e: LambdaEvent<String>) -> Result<usize, Error> { Ok(p(\"x\")) }\n#[tokio::main]\nasync fn main() -> Result<(), Error> { lambda_runtime::run(service_fn(handler)).await }\n",
        expect: "lambda-runtime::",
    },
    // ---- Google Cloud Functions ----
    CloudCase {
        name: "gcp-functions/ruby",
        file: "app.rb",
        src: "require \"functions_framework\"\ndef b(x)\n  x\nend\nFunctionsFramework.http \"hello\" do |request|\n  b(request)\nend\n",
        expect: "gcp-functions::http('hello')",
    },
    CloudCase {
        name: "gcp-functions/php",
        file: "index.php",
        src: "<?php\nuse Google\\CloudFunctions\\FunctionsFramework;\nfunction b($x) { return $x; }\nfunction helloHttp($r) { return b($r); }\nFunctionsFramework::http('helloHttp', 'helloHttp');\n",
        expect: "gcp-functions::",
    },
    CloudCase {
        name: "gcp-functions/go",
        file: "fn.go",
        src: "package fn\nimport \"github.com/GoogleCloudPlatform/functions-framework-go/functions\"\nfunc render() {}\nfunc HelloHTTP() { render() }\nfunc init() { functions.HTTP(\"HelloHTTP\", HelloHTTP) }\n",
        expect: "gcp-functions::http('HelloHTTP')",
    },
    CloudCase {
        name: "gcp-functions/csharp",
        file: "F.cs",
        src: "using Google.Cloud.Functions.Framework;\npublic class F : IHttpFunction {\n  private string R() => \"x\";\n  public System.Threading.Tasks.Task HandleAsync(object ctx) { R(); return null; }\n}\n",
        expect: "gcp-functions::F.HandleAsync",
    },
    // ---- Azure Functions ----
    CloudCase {
        name: "azure-functions/java",
        file: "F.java",
        src: "package com.example;\nimport com.microsoft.azure.functions.annotation.FunctionName;\npublic class F {\n  private String r(String s) { return s; }\n  @FunctionName(\"HttpExample\")\n  public String run(String req) { return r(req); }\n}\n",
        expect: "azure-functions::FunctionName('HttpExample')",
    },
    CloudCase {
        name: "azure-functions/python",
        file: "function_app.py",
        src: "import azure.functions as func\napp = func.FunctionApp()\ndef _l(n):\n    return n\n@app.route(route=\"items\")\ndef get_items(req):\n    return _l(req)\n",
        expect: "azure-functions::route('items')",
    },
    CloudCase {
        name: "azure-functions/typescript",
        file: "index.ts",
        src: "import { app } from '@azure/functions';\nfunction b(x: unknown) { return x; }\napp.http('httpExample', { handler: async (req: unknown) => b(req) });\n",
        expect: "azure-functions::http('httpExample')",
    },
    CloudCase {
        name: "azure-functions/fsharp",
        file: "Fn.fs",
        src: "module Fn\nopen Microsoft.Azure.Functions.Worker\nlet render (s: string) = s\n[<Function(\"HttpTrigger\")>]\nlet run (req: string) = render req\n",
        expect: "azure-functions::Function('HttpTrigger')",
    },
    // ---- Firebase ----
    CloudCase {
        name: "firebase-functions/javascript",
        file: "index.js",
        src: "const functions = require('firebase-functions');\nfunction b(x) { return x; }\nfunction apiHandler(req, res) { res.send(b(req)); }\nexports.api = functions.https.onRequest(apiHandler);\n",
        expect: "firebase-functions::",
    },
    // ---- Cloudflare ----
    CloudCase {
        name: "cloudflare-workers/rust",
        file: "lib.rs",
        src: "use worker::*;\nfn build(p: &str) -> String { p.to_string() }\n#[event(fetch)]\nasync fn main(req: Request) -> Result<Response> { Response::ok(build(\"/\")) }\n",
        expect: "cloudflare-workers::",
    },
    // ---- Deno ----
    CloudCase {
        name: "deno-http/javascript",
        file: "main.js",
        src: "import { serveDir } from \"jsr:@std/http/file-server\";\nfunction b(p) { return p; }\nfunction handler(req) { return b(req.url); }\nDeno.serve(handler);\n",
        expect: "deno-http::handler",
    },
    // The JVM clones and the TypeScript variants: same contract as
    // their Java/JavaScript siblings, different language id.
    CloudCase {
        name: "azure-functions/kotlin",
        file: "F.kt",
        src: "package com.example\nimport com.microsoft.azure.functions.annotation.FunctionName\nclass F {\n  private fun r(s: String) = s\n  @FunctionName(\"HttpExample\")\n  fun run(req: String): String = r(req)\n}\n",
        expect: "azure-functions::",
    },
    CloudCase {
        name: "azure-functions/scala",
        file: "F.scala",
        src: "package com.example\nimport com.microsoft.azure.functions.annotation.FunctionName\nclass F {\n  private def r(s: String): String = s\n  @FunctionName(\"HttpExample\")\n  def run(req: String): String = r(req)\n}\n",
        expect: "azure-functions::",
    },
    CloudCase {
        name: "azure-functions/groovy",
        file: "F.groovy",
        src: "package com.example\nimport com.microsoft.azure.functions.annotation.FunctionName\nclass F {\n  private String r(String s) { s }\n  @FunctionName(\"HttpExample\")\n  String run(String req) { r(req) }\n}\n",
        expect: "azure-functions::",
    },
    CloudCase {
        name: "gcp-functions/kotlin",
        file: "F.kt",
        src: "package com.example\nimport com.google.cloud.functions.HttpFunction\nclass F : HttpFunction {\n  private fun r(s: String) = s\n  override fun service(req: Any, res: Any) { r(\"ok\") }\n}\n",
        expect: "gcp-functions::",
    },
    CloudCase {
        name: "gcp-functions/scala",
        file: "F.scala",
        src: "package com.example\nimport com.google.cloud.functions.HttpFunction\nclass F extends HttpFunction {\n  private def r(s: String): String = s\n  def service(req: Any, res: Any): Unit = { r(\"ok\") }\n}\n",
        expect: "gcp-functions::",
    },
    CloudCase {
        name: "gcp-functions/groovy",
        file: "F.groovy",
        src: "package com.example\nimport com.google.cloud.functions.HttpFunction\nclass F implements HttpFunction {\n  private String r(String s) { s }\n  void service(Object req, Object res) { r(\"ok\") }\n}\n",
        expect: "gcp-functions::",
    },
    CloudCase {
        name: "gcp-functions/typescript",
        file: "index.ts",
        src: "import * as functions from '@google-cloud/functions-framework';\nfunction build(x: unknown) { return x; }\nfunctions.http('helloTs', (req: unknown, res: unknown) => build(req));\n",
        expect: "gcp-functions::http('helloTs')",
    },
    CloudCase {
        name: "aws-lambda/typescript",
        file: "handler.ts",
        src: "import { APIGatewayProxyHandler } from 'aws-lambda';\nfunction v(e: unknown) { return !!e; }\nexport const handler: APIGatewayProxyHandler = async (event) => { v(event); return { statusCode: 200, body: '' }; };\n",
        expect: "aws-lambda::handler",
    },
    CloudCase {
        name: "firebase-functions/typescript",
        file: "index.ts",
        src: "import * as functions from 'firebase-functions';\nfunction b(x: unknown) { return x; }\nfunction apiHandler(req: unknown, res: any) { res.send(b(req)); }\nexport const api = functions.https.onRequest(apiHandler);\n",
        expect: "firebase-functions::",
    },
];

#[test]
fn every_cloud_platform_and_language_pair_enumerates() {
    let mut failures: Vec<String> = Vec::new();
    for case in CLOUD_CASES {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), case.file, case.src);
        let (g, _) = run(tmp.path(), &[]);
        if !g.contains(case.expect) {
            failures.push(format!(
                "{}: expected an entry containing {:?}\n{}",
                case.name, case.expect, g
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cloud pairs did not enumerate:\n\n{}",
        failures.len(),
        CLOUD_CASES.len(),
        failures.join("\n---\n")
    );
}

// ---------------------------------------------------------------------
// Engine and extraction (0.14.0, packet A2)
// ---------------------------------------------------------------------

#[test]
fn an_attribute_over_512_bytes_still_marks_its_entry() {
    // REGRESSION: attribute text over 512 bytes was dropped whole, so a
    // Spring AI `@Tool` with a long description lost its marker and read
    // dead at `high` with a rule naming it.
    let tmp = TempDir::new().unwrap();
    let prose = "Return the weather. ".repeat(40);
    write(
        tmp.path(),
        "src/Tools.java",
        &format!(
            "import org.springframework.ai.tool.annotation.Tool;\n\
             public class Tools {{\n\
             \x20   @Tool(description = \"short one\")\n\
             \x20   String shortTool(String c) {{ return c; }}\n\
             \x20   @Tool(description = \"{prose}\")\n\
             \x20   String longTool(String c) {{ return c; }}\n}}\n"
        ),
    );
    write(
        tmp.path(),
        "rules.toml",
        "[[framework]]\nid = \"t-spring-ai\"\nlanguage = \"java\"\nkind = \"network\"\n\
         detect = [\"org.springframework.ai\"]\nattributes = [\"Tool\"]\nnode = true\n",
    );
    let rules = tmp.path().join("rules.toml");
    let (g, _) = run(tmp.path(), &["--roots", rules.to_str().unwrap()]);
    assert_eq!(
        g.matches("network::t-spring-ai::").count(),
        2,
        "both tools are entries:\n{g}"
    );
}

/// One `[[framework]]` row naming a base type and its entry methods.
fn base_rule(id: &str, lang: &str, detect: &str, base: &str, methods: &str) -> String {
    format!(
        "[[framework]]\nid = \"{id}\"\nlanguage = \"{lang}\"\nkind = \"network\"\n\
         detect = [\"{detect}\"]\nbase_types = [\"{base}\"]\nmethods = [\"{methods}\"]\n\
         node = true\n\n"
    )
}

#[test]
fn base_types_the_plugins_used_to_lose_reach_their_rules() {
    // REGRESSION, one case per language:
    // - TypeScript: an object type argument made `Agent<Env, {…}>` no
    //   type name;
    // - C#: a primary constructor's base arguments did the same;
    // - Ruby: a bodiless intermediate class carried no base, so the
    //   chain `WeatherTool < ApplicationTool < RubyLLM::Tool` ended;
    // - PHP: a base written through a `use … as` alias never matched a
    //   rule naming the class's full name.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.ts",
        "import { Agent } from \"agents\";\n\
         export class MyAgent extends Agent<Env, { n: number; f: () => void }> {\n\
         \x20 onRequest(r: Request) { return r; }\n}\n",
    );
    write(
        tmp.path(),
        "X.cs",
        "using Microsoft.Agents.AI.Workflows;\nnamespace N {\n\
         \x20 class X(string s) : Executor<Msg>(\"id\") {\n\
         \x20   public void HandleAsync(Msg m) { }\n  }\n}\n",
    );
    write(
        tmp.path(),
        "tools.rb",
        "require \"ruby_llm\"\nclass ApplicationTool < RubyLLM::Tool\nend\n\
         class WeatherTool < ApplicationTool\n  def execute(city:)\n    city\n  end\nend\n",
    );
    write(
        tmp.path(),
        "Tool.php",
        "<?php\nnamespace App;\nuse NeuronAI\\Tools\\Tool as BaseTool;\n\
         class Weather extends BaseTool {\n  public function __invoke($city) { return $city; }\n}\n",
    );
    let rules = [
        base_rule("t-agents", "typescript", "agents", "Agent", "onRequest"),
        base_rule(
            "t-maf",
            "csharp",
            "Microsoft.Agents.AI",
            "Executor",
            "HandleAsync",
        ),
        base_rule("t-rubyllm", "ruby", "ruby_llm", "RubyLLM::Tool", "execute"),
        base_rule(
            "t-neuron",
            "php",
            "NeuronAI",
            "NeuronAI\\\\Tools\\\\Tool",
            "__invoke",
        ),
    ]
    .concat();
    write(tmp.path(), "rules.toml", &rules);
    let path = tmp.path().join("rules.toml");
    let (g, _) = run(tmp.path(), &["--roots", path.to_str().unwrap()]);
    for id in ["t-agents", "t-maf", "t-rubyllm", "t-neuron"] {
        assert!(
            g.contains(&format!("network::{id}")),
            "{id} has an entry:\n{g}"
        );
    }
}

#[test]
fn a_keyword_slot_registrar_takes_only_that_slot() {
    // `create_react_agent(m, tools=[t], prompt=p)`: a plain registrar
    // made `my_prompt` a tool; `create_react_agent:tools` takes only what
    // is passed under `tools`, and a second row can give `prompt` its own
    // kind.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        "from langgraph.prebuilt import create_react_agent\n\n\
         def search(q):\n    return q\n\n\
         def my_prompt(state):\n    return \"be nice\"\n\n\
         def build():\n    return create_react_agent(\"m\", tools=[search], prompt=my_prompt)\n",
    );
    write(
        tmp.path(),
        "rules.toml",
        "[[framework]]\nid = \"t-lg\"\nlanguage = \"python\"\nkind = \"network\"\n\
         detect = [\"langgraph\"]\nregistrars = [\"create_react_agent:tools\"]\nnode = true\n\n\
         [[framework]]\nid = \"t-lg-hooks\"\nlanguage = \"python\"\nkind = \"lifecycle\"\n\
         detect = [\"langgraph\"]\nregistrars = [\"create_react_agent:prompt\"]\nnode = true\n",
    );
    let rules = tmp.path().join("rules.toml");
    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .args(["--roots", rules.to_str().unwrap()])
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let names: std::collections::HashMap<String, String> = g["callables"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, c)| {
            (
                id.clone(),
                c["qualified_name"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let mut entries: Vec<(String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| names[e["src"].as_str().unwrap()].contains("framework-entry"))
        .map(|e| {
            let from = &names[e["src"].as_str().unwrap()];
            let kind = if from.contains("::network::") {
                "network"
            } else {
                "lifecycle"
            };
            (kind.to_string(), names[e["dst"].as_str().unwrap()].clone())
        })
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec![
            ("lifecycle".to_string(), "app.my_prompt".to_string()),
            ("network".to_string(), "app.search".to_string()),
        ],
        "{g:#}"
    );
}

#[test]
fn options_object_handlers_are_inline_handlers() {
    // Method shorthand (plain, async, generator), a handler behind one
    // wrapping call, and handlers one level down (a nested object, an
    // array of objects, a shorthand property naming a function) were no
    // handlers, and shorthand read dead with a rule naming `tool`.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "agent.ts",
        "import { tool } from \"ai\";\n\
         function a() { return 1; }\nfunction b() { return 2; }\nfunction c() { return 3; }\n\
         function d() { return 4; }\nfunction e() { return 5; }\nfunction onStart() { return 6; }\n\
         function wrap(f: any) { return f; }\n\
         export const t1 = tool({ description: \"x\", async execute() { return a(); } });\n\
         export const t2 = tool({ async *execute() { yield b(); } });\n\
         export const t3 = tool({ execute: wrap(async () => c()) });\n\
         export const t4 = tool({ lifecycle: { onEnd: () => d() } });\n\
         export const t5 = tool({ functions: [{ func: () => e() }] });\n\
         export const t6 = tool({ lifecycle: { onStart } });\n",
    );
    write(
        tmp.path(),
        "rules.toml",
        "[[framework]]\nid = \"t-ai\"\nlanguage = \"typescript\"\nkind = \"network\"\n\
         detect = [\"ai\"]\nregistrars = [\"tool\"]\nnode = true\n",
    );
    let rules = tmp.path().join("rules.toml");
    let (g, _) = run(tmp.path(), &["--roots", rules.to_str().unwrap()]);
    assert_eq!(
        g.matches("network::t-ai::").count(),
        6,
        "one entry per handler:\n{g}"
    );
    let report = tmp.path().join("dead.json");
    cgg()
        .arg(tmp.path())
        .args(["--roots", rules.to_str().unwrap()])
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .args(["--dead-code-format", "json", "--dead-code-report"])
        .arg(&report)
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    let dead: Vec<&str> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap())
        .collect();
    assert!(
        dead.is_empty(),
        "every handler and its callees are live: {dead:?}"
    );
}

/// Dead-code findings (qualified names) of a run over `dir`.
fn dead_findings(dir: &Path, extra: &[&str]) -> Vec<String> {
    let report = dir.join("dead.json");
    cgg()
        .arg(dir)
        .args(["--dead-code", "--dead-code-confidence", "low"])
        .args(["--dead-code-format", "json", "--dead-code-report"])
        .arg(&report)
        .args(extra)
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).unwrap()).unwrap();
    v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_minted_closure_no_rule_claims_keeps_its_callees_live() {
    // REGRESSION: an inline closure handed to a registrar verb is minted
    // as `handler_at_<line>` whether or not a rule claims it. Unclaimed,
    // nothing reached it, and its callees read dead: written inline they
    // belonged to the enclosing function (or the top level). One case
    // per language that mints them.
    let tmp = TempDir::new().unwrap();
    let d = tmp.path();
    write(
        d,
        "a.js",
        "function helperJs() { return 1; }\nexport function setup(bus) {\n\
         \x20 bus.get(\"/y\", () => helperJs());\n}\nsetup({});\n\
         app.post(\"/z\", () => helperJs2());\nfunction helperJs2() { return 2; }\n",
    );
    write(
        d,
        "main.go",
        "package main\n\nfunc renderGo() string { return \"x\" }\n\n\
         func setup(m *Mux) {\n\tm.Get(\"/\", func() string {\n\t\treturn renderGo()\n\t})\n}\n\n\
         func main() { setup(nil) }\n",
    );
    write(
        d,
        "app.rb",
        "def helper_rb\n  1\nend\n\ndef boot_rb\n  2\nend\n\nboot_rb()\n\n\
         get \"/x\" do\n  helper_rb\nend\n",
    );
    write(
        d,
        "main.swift",
        "func greetSw(_ n: String) -> String { n }\nfunc bootSw() {}\nbootSw()\n\
         Lambda.run { (c, n: String, cb) in\n  greetSw(n)\n}\n",
    );
    write(
        d,
        "app.clj",
        "(ns app)\n(defn home-page [] \"hi\")\n(defn routes [] (GET \"/\" [] (home-page)))\n(routes)\n",
    );
    write(
        d,
        "router.ex",
        "defmodule MyApp.Router do\n  def helper_ex, do: :ok\n  def boot_ex, do: :ok\n\n\
         \x20 boot_ex()\n\n  get \"/health\" do\n    helper_ex()\n  end\nend\n",
    );
    let dead = dead_findings(d, &[]);
    assert!(dead.is_empty(), "every helper is live: {dead:?}");
}

#[test]
fn a_minted_closure_sees_the_enclosing_functions_parameter_types() {
    // REGRESSION: a minted closure was its own scope for type
    // propagation, so the enclosing function's typed parameter `files`
    // was unknown inside it and the call bound by name at `medium`.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.go",
        "package main\n\ntype Files struct{}\n\nfunc (f *Files) CreateVersion() {}\n\n\
         type Other struct{}\n\nfunc (o *Other) CreateVersion() {}\n\n\
         func setup(m *Mux, files *Files) {\n\tm.Get(\"/\", func() string {\n\
         \t\tfiles.CreateVersion()\n\t\treturn \"\"\n\t})\n}\n",
    );
    write(
        tmp.path(),
        "a.ts",
        "class Files { createVersion() {} }\nclass Other { createVersion() {} }\n\
         export function setup(app, files: Files) {\n  app.get(\"/\", () => { files.createVersion(); });\n}\n",
    );
    let out = tmp.path().join("g.json");
    cgg()
        .args(["-t", "json", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let g: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let qn = |id: &serde_json::Value| {
        g["callables"][id.as_str().unwrap()]["qualified_name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mut got: Vec<(String, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| qn(&e["src"]).contains("handler_at_"))
        .map(|e| (qn(&e["dst"]), e["confidence"].as_str().unwrap().to_string()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("Files.createVersion".to_string(), "high".to_string()),
            ("main.Files.CreateVersion".to_string(), "high".to_string()),
        ]
    );
}

// ---------------------------------------------------------------------
// 0.14.0 rows: Elixir, Erlang, Haskell, OCaml and Solidity
// ---------------------------------------------------------------------
//
// One case per new rule: the smallest source carrying the framework's
// real shape, and the coverage line it must produce. A count, not a
// substring of the graph: `node: false` rules mint no node to look for.
struct RowCase {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    expect: &'static str,
}

const ROW_CASES: &[RowCase] = &[
    RowCase {
        name: "oban",
        files: &[(
            "lib/w.ex",
            "defmodule MyApp.W do\n  use Oban.Worker\n  def perform(_job), do: helper()\n  def backoff(_a), do: 5\n  defp helper, do: :ok\nend\n",
        )],
        expect: "oban (queue, 2 entries)",
    },
    RowCase {
        name: "ecto-migration",
        files: &[(
            "priv/repo/migrations/20200101_add.exs",
            "defmodule MyApp.Repo.Migrations.Add do\n  use Ecto.Migration\n  def up do\n    create table(:x)\n  end\n  def down do\n    drop table(:x)\n  end\nend\n",
        )],
        expect: "ecto-migration (lifecycle, 2 entries)",
    },
    RowCase {
        name: "ecto-type",
        files: &[(
            "lib/t.ex",
            "defmodule MyApp.T do\n  use Ecto.Type\n  def type, do: :string\n  def cast(x), do: {:ok, x}\n  def load(x), do: {:ok, x}\n  def dump(x), do: {:ok, x}\nend\n",
        )],
        expect: "ecto-type (lifecycle, 4 entries)",
    },
    RowCase {
        name: "broadway",
        files: &[(
            "lib/p.ex",
            "defmodule MyApp.P do\n  use Broadway\n  def handle_message(_p, m, _c), do: m\n  def handle_batch(_b, ms, _i, _c), do: ms\nend\n",
        )],
        expect: "broadway (queue, 2 entries)",
    },
    RowCase {
        name: "genstage",
        files: &[(
            "lib/s.ex",
            "defmodule MyApp.S do\n  use GenStage\n  def handle_demand(d, s), do: {:noreply, [d], s}\n  def handle_events(e, _f, s), do: {:noreply, e, s}\nend\n",
        )],
        expect: "genstage (lifecycle, 2 entries)",
    },
    RowCase {
        name: "plug/handle_errors",
        files: &[(
            "lib/r.ex",
            "defmodule MyApp.Router do\n  use Plug.ErrorHandler\n  def handle_errors(conn, _e), do: conn\nend\n",
        )],
        expect: "plug (network, 1 entry)",
    },
    RowCase {
        name: "rebar3-provider",
        files: &[(
            "src/p.erl",
            "-module(p).\n-behaviour(provider).\n-export([init/1, do/1, format_error/1]).\ninit(S) -> {ok, S}.\ndo(S) -> {ok, S}.\nformat_error(R) -> R.\n",
        )],
        expect: "rebar3-provider (cli, 2 entries)",
    },
    RowCase {
        name: "quickcheck",
        files: &[(
            "src/A.hs",
            "module A where\nimport Test.QuickCheck\nprop_rev xs = reverse (reverse xs) == (xs :: [Int])\nhelper = 1\nrunTests = $quickCheckAll\n",
        )],
        expect: "quickcheck (test, 1 entry)",
    },
    RowCase {
        name: "hedgehog",
        files: &[(
            "test/B.hs",
            "module B where\nimport Hedgehog\nprop_x = property success\nhprop_y = property success\ntests = checkParallel $$(discover)\n",
        )],
        expect: "hedgehog (test, 2 entries)",
    },
    RowCase {
        name: "ihp",
        files: &[
            (
                "Web/Controller/Posts.hs",
                "module Web.Controller.Posts where\nimport IHP.ControllerPrelude\ninstance Controller PostsController where\n    beforeAction = ensureIsUser\n    action PostsAction = render IndexView\nhelperUnused x = x\n",
            ),
            (
                "Web/View/Posts/Index.hs",
                "module Web.View.Posts.Index where\nimport IHP.ViewPrelude\ninstance View IndexView where\n    html IndexView = [hsx|<h1>Posts</h1>|]\n",
            ),
        ],
        expect: "ihp (network, 3 entries)",
    },
    RowCase {
        name: "yesod",
        files: &[(
            "src/Main.hs",
            "module Main where\nimport Yesod\ngetHomeR = defaultLayout mempty\npostUserR = undefined\nhandleStaticR = undefined\nsdistR = 1\ngetter = 2\nmain = warp 3000 App\n",
        )],
        expect: "yesod (network, 3 entries)",
    },
    RowCase {
        name: "mirage",
        files: &[
            (
                "config.ml",
                "open Mirage\nlet main = main \"Unikernel.Main\" (stackv4v6 @-> job)\nlet () = register \"hello\" [ main $ generic_stackv4v6 default_network ]\n",
            ),
            (
                "unikernel.ml",
                "module Main (S : Tcpip.Stack.V4V6) = struct\n  let helper x = x + 1\n  let start s = ignore (helper 1); ignore s; Lwt.return_unit\nend\n",
            ),
        ],
        expect: "mirage (lifecycle, 1 entry)",
    },
    RowCase {
        name: "chainlink",
        files: &[(
            "C.sol",
            "import {CCIPReceiver} from \"@chainlink/contracts-ccip/src/v0.8/ccip/applications/CCIPReceiver.sol\";\n\
             contract C is CCIPReceiver {\n  function _ccipReceive(bytes memory m) internal override { _apply(m); }\n  function _apply(bytes memory m) internal {}\n}\n",
        )],
        expect: "chainlink (network, 1 entry)",
    },
];

#[test]
fn every_new_functional_and_contract_row_enumerates() {
    let mut failures: Vec<String> = Vec::new();
    for case in ROW_CASES {
        let tmp = TempDir::new().unwrap();
        for (file, src) in case.files {
            write(tmp.path(), file, src);
        }
        let (_, err) = run(tmp.path(), &["--framework-coverage"]);
        if !err.contains(case.expect) {
            failures.push(format!(
                "{}: expected {:?}\n{}",
                case.name, case.expect, err
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rows did not enumerate:\n\n{}",
        failures.len(),
        ROW_CASES.len(),
        failures.join("\n---\n")
    );
}

/// Run `--dead-code` at medium confidence and return the reported
/// unreferenced count line.
fn dead_count(dir: &Path) -> String {
    let out = dir.join("d.mmd");
    let assert = cgg()
        .args(["--dead-code", "--dead-code-confidence", "medium", "-o"])
        .arg(&out)
        .arg(dir)
        .assert()
        .success();
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
}

/// BACKLOG P1: a Solidity base's call to its `virtual` hook runs the
/// most-derived override, so the override and what it calls are live.
#[test]
fn a_solidity_override_of_an_in_tree_virtual_hook_is_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Base.sol",
        "pragma solidity ^0.8.20;\n\
         abstract contract Base {\n  function run() external { _hook(); }\n  function _hook() internal virtual {}\n}\n\
         contract Impl is Base {\n  function _hook() internal override { _work(); }\n  function _work() internal {}\n}\n",
    );
    let err = dead_count(tmp.path());
    assert!(err.contains("0 callable(s) marked unreferenced"), "{err}");
}

/// BACKLOG P1: `fun ?MODULE:f/N`, `{?MODULE, f, A}` (a hook appends its
/// own arguments, so `A` is not `f`'s arity), `{?MODULE, f}` and an
/// `argparse` `handler => {?MODULE, f}` name this module's functions.
#[test]
fn erlang_module_macro_callbacks_are_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/hk.erl",
        "-module(hk).\n-export([start/0, on_pub/2, on_conn/2, on_fun/2, cli/0, do_list/1]).\n\
         start() ->\n    emqx_hooks:add('message.publish', {?MODULE, on_pub, []}, 10),\n\
         \x20   emqx_hooks:add('client.connect', {?MODULE, on_conn}, 10),\n\
         \x20   telemetry:attach(<<\"id\">>, [a], fun ?MODULE:on_fun/2, #{}),\n    cli().\n\
         on_pub(_, _) -> ok.\non_conn(_, _) -> ok.\non_fun(_, _) -> ok.\n\
         cli() -> #{commands => #{\"list\" => #{handler => {?MODULE, do_list}}}}.\n\
         do_list(_) -> ok.\n",
    );
    let err = dead_count(tmp.path());
    assert!(err.contains("0 callable(s) marked unreferenced"), "{err}");
}

/// BACKLOG P1: a proto rpc is a root, so the implementation the
/// descriptor linker finds (through a declared base here) and its
/// callees are live.
#[test]
fn a_proto_rpc_keeps_its_implementation_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "proto/svc.proto",
        "syntax = \"proto3\";\nservice Suggestion {\n  rpc GetSuggestions (Req) returns (Resp);\n}\n\
         message Req { string a = 1; }\nmessage Resp { string b = 1; }\n",
    );
    write(
        tmp.path(),
        "svc/hyperopt.py",
        "import grpc\nfrom pkg import api_pb2_grpc\n\n\
         class HyperoptService(api_pb2_grpc.SuggestionServicer):\n\
         \x20   def GetSuggestions(self, request, context):\n        return _plan(request)\n\n\
         def _plan(r):\n    return r\n",
    );
    let err = dead_count(tmp.path());
    assert!(err.contains("0 callable(s) marked unreferenced"), "{err}");
}

/// BACKLOG P1 (connexion): an `operationId` under
/// `x-openapi-router-controller` names its Python handler, which is live.
#[test]
fn a_connexion_operation_id_keeps_its_handler_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "openapi.yaml",
        "openapi: 3.0.0\ninfo:\n  title: t\n  version: '1'\npaths:\n  /items:\n\
         \x20   get:\n      x-openapi-router-controller: app.api\n      operationId: list_items\n\
         \x20     responses:\n        '200':\n          description: ok\n",
    );
    write(
        tmp.path(),
        "app/api.py",
        "def list_items():\n    return _load()\n\n\ndef _load():\n    return []\n",
    );
    write(tmp.path(), "app/__init__.py", "");
    let err = dead_count(tmp.path());
    assert!(err.contains("0 callable(s) marked unreferenced"), "{err}");
}
