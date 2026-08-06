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
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "m.f90", "subroutine work()\nend subroutine\n");
    write(
        tmp.path(),
        "svc.py",
        "from flask import Flask\napp = Flask(__name__)\n\
         @app.route(\"/x\")\ndef x():\n    return 1\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    assert!(err.contains("no framework rules"), "{err}");
    assert!(err.contains("fortran"), "{err}");
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
