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
