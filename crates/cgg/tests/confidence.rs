//! WS19: every `high` edge must be true. End-to-end fixtures, one per
//! affected language, for the five ways a guess used to be reported as
//! certain — and a near miss for each that must stay as it was.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// One edge: caller, callee, the callee's file, and the confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edge {
    src: String,
    dst: String,
    dst_file: String,
    confidence: String,
}

fn graph(files: &[(&str, &str)]) -> Vec<Edge> {
    let tmp = TempDir::new().unwrap();
    for (name, body) in files {
        let p = tmp.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
    }
    edges_of(tmp.path())
}

fn edges_of(dir: &Path) -> Vec<Edge> {
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .args(["-t", "json", "--no-entry-nodes"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let g: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let qn = |id: &serde_json::Value| -> (String, String) {
        let c = &g["callables"][id.as_str().unwrap()];
        let file = c["file"].as_str().unwrap();
        (
            c["qualified_name"].as_str().unwrap().to_string(),
            g["files"][file]["path"]
                .as_str()
                .unwrap()
                .trim_start_matches(dir.to_str().unwrap())
                .trim_start_matches('/')
                .to_string(),
        )
    };
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let (src, _) = qn(&e["src"]);
            let (dst, dst_file) = qn(&e["dst"]);
            Edge {
                src,
                dst,
                dst_file,
                confidence: e["confidence"].as_str().unwrap().to_string(),
            }
        })
        .collect()
}

/// The confidences of the edges from `src` to `dst`.
fn conf(edges: &[Edge], src: &str, dst: &str) -> Vec<String> {
    edges
        .iter()
        .filter(|e| e.src == src && e.dst == dst)
        .map(|e| e.confidence.clone())
        .collect()
}

fn high() -> Vec<String> {
    vec!["high".to_string()]
}

fn medium() -> Vec<String> {
    vec!["medium".to_string()]
}

/// Gap 1: a receiver typed only by a heuristic (`service` spelled like a
/// type the file declares) binds its one same-file candidate at medium.
#[test]
fn a_guessed_receiver_type_binds_medium_in_every_language() {
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "a.py",
            "class Service:\n    def run(self):\n        pass\n\ndef main(service):\n    service.run()\n",
            "a.main",
            "a.Service.run",
        ),
        (
            "a.js",
            "class Service {\n  run() {}\n}\nfunction main(service) {\n  service.run();\n}\n",
            "main",
            "Service.run",
        ),
        (
            "a.ts",
            "class Service {\n  run(): void {}\n}\nfunction main(service: any) {\n  service.run();\n}\n",
            "main",
            "Service.run",
        ),
        (
            "a.go",
            "package a\n\ntype Service struct{}\n\nfunc (s *Service) Run() {}\n\nfunc main() {\n\tservice := mk()\n\tservice.Run()\n}\n",
            "a.main",
            "a.Service.Run",
        ),
        (
            "A.java",
            "class Service {\n  void run() {}\n  static void main() {\n    var service = mk();\n    service.run();\n  }\n}\n",
            "Service.main",
            "Service.run",
        ),
        (
            "a.kt",
            "class Service {\n  fun run() {}\n}\nfun main() {\n  val service = mk()\n  service.run()\n}\n",
            "main",
            "Service.run",
        ),
        (
            "a.cpp",
            "class Service {\npublic:\n  void run() {}\n};\nvoid main2() {\n  auto service = mk();\n  service.run();\n}\n",
            "main2",
            "Service::run",
        ),
        (
            "A.cs",
            "class Service {\n  void Run() {}\n  static void Main() {\n    var service = Mk();\n    service.Run();\n  }\n}\n",
            "Service.Main",
            "Service.Run",
        ),
        (
            "a.rb",
            "class Service\n  def run\n  end\nend\ndef main(service)\n  service.run\nend\n",
            "main",
            "Service::run",
        ),
        (
            "a.swift",
            "class Service {\n  func run() {}\n}\nfunc main() {\n  let service = mk()\n  service.run()\n}\n",
            "main",
            "Service.run",
        ),
        (
            "a.lua",
            "Service = {}\nfunction Service:run()\nend\nfunction main(service)\n  service:run()\nend\n",
            "main",
            "Service:run",
        ),
        (
            "a.dart",
            "class Service {\n  void run() {}\n}\nvoid main() {\n  var service = mk();\n  service.run();\n}\n",
            "main",
            "Service::run",
        ),
        (
            "a.scala",
            "class Service {\n  def run(): Unit = {}\n}\nobject M {\n  def main(): Unit = {\n    val service = mk()\n    service.run()\n  }\n}\n",
            "M::main",
            "Service::run",
        ),
        (
            "a.rs",
            "struct Service;\nimpl Service {\n    fn run(&self) {}\n}\nfn main() {\n    let service = mk();\n    service.run();\n}\n",
            "crate::main",
            "crate::Service::run",
        ),
        (
            "a.m",
            "@interface Service\n- (void)run;\n@end\n@implementation Service\n- (void)run {}\n@end\nvoid main2(id service) {\n  [service run];\n}\n",
            "main2",
            "Service::run",
        ),
        (
            "a.groovy",
            "class Service {\n  void run() {}\n}\ndef main() {\n  def service = mk()\n  service.run()\n}\n",
            "main",
            "Service.run",
        ),
    ];
    for (file, body, src, dst) in cases {
        let edges = graph(&[(file, body)]);
        assert_eq!(conf(&edges, src, dst), medium(), "{file}: {edges:#?}");
    }
    // Near miss: a declared type is known, not guessed.
    let edges = graph(&[(
        "a.py",
        "class Service:\n    def run(self):\n        pass\n\ndef main(service: Service):\n    service.run()\n",
    )]);
    assert_eq!(
        conf(&edges, "a.main", "a.Service.run"),
        high(),
        "{edges:#?}"
    );
    let edges = graph(&[(
        "A.java",
        "class Service {\n  void run() {}\n  static void main() {\n    Service service = mk();\n    service.run();\n  }\n}\n",
    )]);
    assert_eq!(
        conf(&edges, "Service.main", "Service.run"),
        high(),
        "{edges:#?}"
    );
}

/// Gap 2: a lone function nested in another is not visible by name in
/// some other function. The binding is kept (a returned closure may be
/// called through a same-named local) but is medium; inside its parent
/// it stays high. Dart records no callable for a local function, so it
/// has no such edge either way.
#[test]
fn a_lone_nested_function_is_medium_outside_its_parent() {
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "a.py",
            "def outer():\n    def helper():\n        pass\n    helper()\n\ndef other():\n    helper()\n",
            "a.outer",
            "a.outer.helper",
        ),
        (
            "a.js",
            "function outer(a) {\n  function helper() {}\n  helper();\n}\nfunction other(b) {\n  helper();\n}\n",
            "outer",
            "helper",
        ),
        (
            "a.ts",
            "function outer(a: number): void {\n  function helper(): void {}\n  helper();\n}\nfunction other(b: number): void {\n  helper();\n}\n",
            "outer",
            "helper",
        ),
        (
            "a.lua",
            "function outer()\n  local function helper()\n  end\n  helper()\nend\nfunction other()\n  helper()\nend\n",
            "outer",
            "helper",
        ),
        (
            "a.ml",
            "let outer () =\n  let helper () = () in\n  helper ()\n\nlet other () =\n  helper ()\n",
            "outer",
            "helper",
        ),
        (
            "a.fs",
            "module A\nlet outer () =\n    let helper () = ()\n    helper ()\n\nlet other () =\n    helper ()\n",
            "outer",
            "helper",
        ),
        (
            "a.scala",
            "object A {\n  def outer(): Unit = {\n    def helper(): Unit = {}\n    helper()\n  }\n  def other(): Unit = {\n    helper()\n  }\n}\n",
            "A::outer",
            "A::helper",
        ),
        (
            "a.kt",
            "fun outer() {\n    fun helper() {}\n    helper()\n}\nfun other() {\n    helper()\n}\n",
            "outer",
            "helper",
        ),
        (
            "a.swift",
            "func outer() {\n    func helper() {}\n    helper()\n}\nfunc other() {\n    helper()\n}\n",
            "outer",
            "helper",
        ),
        (
            "a.rs",
            "fn outer() {\n    fn helper() {}\n    helper();\n}\nfn other() {\n    helper();\n}\n",
            "crate::outer",
            "crate::helper",
        ),
        (
            "A.hs",
            "module A where\nouter :: Int -> Int\nouter x = helper x\n  where\n    helper y = y + 1\n\nother :: Int -> Int\nother z = helper z\n",
            "A.outer",
            "A.helper",
        ),
        (
            "a.jl",
            "function outer()\n    function helper()\n    end\n    helper()\nend\nfunction other()\n    helper()\nend\n",
            "outer",
            "helper",
        ),
    ];
    for (file, body, outer, helper) in cases {
        let edges = graph(&[(file, body)]);
        let other = outer.replace("outer", "other");
        assert_eq!(conf(&edges, outer, helper), high(), "{file}: {edges:#?}");
        assert_eq!(conf(&edges, &other, helper), medium(), "{file}: {edges:#?}");
    }
}

/// Gap 3: a lambda or closure parameter shadows like a function's
/// parameter: a function nested in some other function is never the
/// callee of a call through it. Near miss: the nested function's own
/// parent still calls it at high.
#[test]
fn a_closure_parameter_is_never_a_nested_function_elsewhere() {
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "a.py",
            "def outer():\n    def go():\n        pass\n    go()\n\ndef other():\n    f = lambda go: go()\n    return f\n",
            "a.outer",
            "a.outer.go",
        ),
        (
            "a.js",
            "function outer(a) {\n  function go() {}\n  go();\n}\nfunction other(b) {\n  const f = (go) => go();\n  return f;\n}\n",
            "outer",
            "go",
        ),
        (
            "a.ts",
            "function outer(a: number): void {\n  function go(): void {}\n  go();\n}\nfunction other(b: number): void {\n  const f = (go: () => void) => go();\n  f(() => {});\n}\n",
            "outer",
            "go",
        ),
        (
            "a.rs",
            "fn outer() {\n    fn go() {}\n    go();\n}\nfn other() {\n    let f = |go: fn()| go();\n    f(outer);\n}\n",
            "crate::outer",
            "crate::go",
        ),
        (
            "a.ml",
            "let outer () =\n  let go () = () in\n  go ()\n\nlet other () =\n  let f = fun go -> go () in\n  f (fun () -> ())\n",
            "outer",
            "go",
        ),
        (
            "a.kt",
            "fun outer() {\n    fun go() {}\n    go()\n}\nfun other() {\n    val f = { go: () -> Unit -> go() }\n    f {}\n}\n",
            "outer",
            "go",
        ),
        (
            "a.swift",
            "func outer() {\n    func go() {}\n    go()\n}\nfunc other() {\n    let f = { (go: () -> Void) in go() }\n    f({})\n}\n",
            "outer",
            "go",
        ),
        (
            "a.scala",
            "object A {\n  def outer(): Unit = {\n    def go(): Unit = {}\n    go()\n  }\n  def other(): Unit = {\n    val f = (go: () => Unit) => go()\n    f(() => ())\n  }\n}\n",
            "A::outer",
            "A::go",
        ),
        (
            "a.lua",
            "function outer()\n  local function go()\n  end\n  go()\nend\nfunction other()\n  local f = function(go) go() end\n  f(outer)\nend\n",
            "outer",
            "go",
        ),
    ];
    for (file, body, outer, go) in cases {
        let edges = graph(&[(file, body)]);
        let into_go: Vec<&Edge> = edges.iter().filter(|e| e.dst == *go).collect();
        assert_eq!(into_go.len(), 1, "{file}: {edges:#?}");
        assert_eq!(conf(&edges, outer, go), high(), "{file}: {edges:#?}");
    }
}

/// Gap 4: a function's return type types a receiver only from the
/// receiver's own language, and only when every definition of the name
/// agrees. Python's `make` returns a `Worker` (unannotated, or
/// annotated in one copy and `Service` in another); a Swift `make` or
/// the other Python copy used to type `service` as `Service` and bind
/// `Service.run`.
#[test]
fn a_return_type_from_another_language_or_copy_does_not_type_a_receiver() {
    let py = |factory: &'static str| -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "py/svc.py",
                "class Service:\n    def run(self):\n        pass\n",
            ),
            (
                "py/wrk.py",
                "class Worker:\n    def run(self):\n        pass\n",
            ),
            ("py/factory.py", factory),
            (
                "py/main.py",
                "from factory import make\nfrom svc import Service\n\ndef main():\n    service = make()\n    service.run()\n",
            ),
        ]
    };
    let unannotated = "from wrk import Worker\n\ndef make():\n    return Worker()\n";
    let mut files = py(unannotated);
    files.push((
        "swift/a.swift",
        "class Service {\n    func run() {}\n}\nfunc make() -> Service {\n    return Service()\n}\n",
    ));
    let edges = graph(&files);
    assert!(
        conf(&edges, "main.main", "svc.Service.run").is_empty(),
        "{edges:#?}"
    );
    let mut files =
        py("from wrk import Worker\n\ndef make() -> Worker:\n    return Worker()\n");
    files.push((
        "py/aaa_other.py",
        "from svc import Service\n\ndef make() -> Service:\n    return Service()\n",
    ));
    let edges = graph(&files);
    assert!(
        conf(&edges, "main.main", "svc.Service.run").is_empty(),
        "{edges:#?}"
    );
    // Near miss: one Python `make`, declared to return `Service`.
    let edges = graph(&py(
        "from svc import Service\n\ndef make() -> Service:\n    return Service()\n",
    ));
    assert_eq!(
        conf(&edges, "main.main", "svc.Service.run"),
        medium(),
        "{edges:#?}"
    );
}

/// Gap 5: an include-style import names the file whose definition the
/// call reaches. With a same-named decoy elsewhere (`zz/`, which used to
/// win as the last copy indexed) the call binds the included copy.
#[test]
fn an_include_binds_the_copy_in_the_file_it_names() {
    // (main file, main body, included path, included body, caller, callee)
    let cases: &[(&str, &str, &str, &str, &str, &str)] = &[
        (
            "main.sh",
            "source ./lib/one.sh\nmain() {\n  dup_fn\n}\n",
            "one.sh",
            "dup_fn() {\n  echo x\n}\n",
            "main",
            "dup_fn",
        ),
        (
            "main.rb",
            "require_relative 'lib/two'\ndef main\n  dup_fn\nend\n",
            "two.rb",
            "def dup_fn\n  1\nend\n",
            "main",
            "dup_fn",
        ),
        (
            "main.R",
            "source(\"lib/u2.R\")\nmainr <- function() {\n  dupr()\n}\n",
            "u2.R",
            "dupr <- function() {\n  1\n}\n",
            "mainr",
            "dupr",
        ),
        (
            "main.ps1",
            ". ./lib/h2.ps1\nfunction Main {\n  Get-Dup\n}\n",
            "h2.ps1",
            "function Get-Dup {\n  1\n}\n",
            "Main",
            "Get-Dup",
        ),
        (
            "C.sol",
            "pragma solidity ^0.8.0;\nimport \"./lib/B.sol\";\ncontract C {\n  function m() public { dsol(); }\n}\n",
            "B.sol",
            "pragma solidity ^0.8.0;\nfunction dsol() pure {}\n",
            "C.m",
            "dsol",
        ),
        (
            "a.bzl",
            "load(\"//lib:b.bzl\", \"dup\")\ndef main():\n    dup()\n",
            "b.bzl",
            "def dup():\n    pass\n",
            "main",
            "dup",
        ),
        (
            "main.cmake",
            "include(lib/helpers.cmake)\nfunction(main_fn)\n  dup_cm()\nendfunction()\n",
            "helpers.cmake",
            "function(dup_cm)\n  message(\"x\")\nendfunction()\n",
            "main_fn",
            "dup_cm",
        ),
        (
            "main.lua",
            "require(\"lib.m3\")\nfunction main()\n  dupl()\nend\n",
            "m3.lua",
            "function dupl()\nend\n",
            "main",
            "dupl",
        ),
    ];
    for (main, body, inc, inc_body, caller, callee) in cases {
        let lib = format!("lib/{inc}");
        let decoy = format!("zz/{inc}");
        let edges = graph(&[(main, body), (&lib, inc_body), (&decoy, inc_body)]);
        let got: Vec<(&str, &str)> = edges
            .iter()
            .filter(|e| e.src == *caller && e.dst == *callee)
            .map(|e| (e.dst_file.as_str(), e.confidence.as_str()))
            .collect();
        assert_eq!(got, vec![(lib.as_str(), "high")], "{main}: {edges:#?}");
        // Near miss: no decoy, the same binding.
        let edges = graph(&[(main, body), (&lib, inc_body)]);
        assert_eq!(conf(&edges, caller, callee), high(), "{main}: {edges:#?}");
    }
}

/// Gap 5: an include written as a bare file name that two files of the
/// tree end with could name either; their same-named definitions are a
/// medium fan-out, never one picked at high.
#[test]
fn an_include_that_names_no_single_file_is_medium_fan_out() {
    let cases: &[(&str, &str, &str, &str, &str, &str)] = &[
        (
            "main.rb",
            "require 'two'\ndef main\n  dup_fn\nend\n",
            "two.rb",
            "def dup_fn\n  1\nend\n",
            "main",
            "dup_fn",
        ),
        (
            "main.sh",
            "source one.sh\nmain() {\n  dup_fn\n}\n",
            "one.sh",
            "dup_fn() {\n  echo x\n}\n",
            "main",
            "dup_fn",
        ),
    ];
    for (main, body, inc, inc_body, caller, callee) in cases {
        let lib = format!("lib/{inc}");
        let decoy = format!("zz/{inc}");
        let edges = graph(&[(main, body), (&lib, inc_body), (&decoy, inc_body)]);
        let mut got: Vec<(&str, &str)> = edges
            .iter()
            .filter(|e| e.src == *caller && e.dst == *callee)
            .map(|e| (e.dst_file.as_str(), e.confidence.as_str()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![(lib.as_str(), "medium"), (decoy.as_str(), "medium")],
            "{main}: {edges:#?}"
        );
    }
}

/// Gap 5, Lua: a `local function` is private to its file, so requiring
/// the file never puts it in the caller's scope: a same-named call is at
/// most a by-name guess. Near miss: a global
/// function of the required file is still reached, at high.
#[test]
fn a_required_lua_files_local_function_is_not_in_scope() {
    let edges = graph(&[
        (
            "main.lua",
            "require(\"lib.m\")\nfunction main()\n  log()\n  glob()\nend\n",
        ),
        (
            "lib/m.lua",
            "local function log()\nend\nfunction glob()\nend\n",
        ),
    ]);
    // At most a by-name guess, never the include's certainty.
    assert!(
        !conf(&edges, "main", "log").contains(&"high".to_string()),
        "{edges:#?}"
    );
    assert_eq!(conf(&edges, "main", "glob"), high(), "{edges:#?}");
}

/// Lua: a file-scope `local log = ngx.log` is a value the file holds; a
/// bare `log(…)` is never a required module's `_M.log` (it was, at high).
/// Near miss: `local feed = t.feed` with `t = require(…)` is another name
/// for the module's member and still binds it at high.
#[test]
fn a_lua_file_scope_local_shadows_but_a_member_alias_does_not() {
    let edges = graph(&[
        (
            "kong/cmd/log.lua",
            "local _M = {}\nfunction _M.log(a)\nend\nfunction _M.feed(a)\nend\nreturn _M\n",
        ),
        (
            "kong/conn.lua",
            "local logger = require \"kong.cmd.log\"\nlocal log = ngx.log\nlocal t = require(\"kong.cmd.log\")\nlocal feed = t.feed\nlocal function run()\n  log(1, \"x\")\n  feed(2)\nend\n",
        ),
    ]);
    assert!(conf(&edges, "run", "_M.log").is_empty(), "{edges:#?}");
    assert_eq!(conf(&edges, "run", "_M.feed"), high(), "{edges:#?}");
}

/// A Kotlin property's declared type — a class-body `lateinit var`, a
/// `val x: T = …`, a constructor `val` parameter — is a declaration, not
/// a guess, scoped to its class: two classes of one file declaring the
/// same name with different types each bind their own. Before, the
/// file-wide reading typed both as the last one, and called it a guess.
#[test]
fn a_kotlin_property_type_is_declared_and_scoped_to_its_class() {
    let edges = graph(&[
        (
            "T.kt",
            "package a.b\nclass Vm { fun onQ() {} }\nclass Wm { fun onQ() {} }\n",
        ),
        (
            "U.kt",
            "package a.b\nclass A {\n  lateinit var vm: Vm\n  fun t() { vm.onQ() }\n}\nclass B {\n  lateinit var vm: Wm\n  fun t() { vm.onQ() }\n}\nclass C(val cv: Vm) {\n  fun t() { cv.onQ() }\n}\nclass D {\n  val dv: Vm = Vm()\n  fun t() { dv.onQ() }\n}\n",
        ),
    ]);
    for (src, dst) in [
        ("a.b.A.t", "a.b.Vm.onQ"),
        ("a.b.B.t", "a.b.Wm.onQ"),
        ("a.b.C.t", "a.b.Vm.onQ"),
        ("a.b.D.t", "a.b.Vm.onQ"),
    ] {
        let to: Vec<&Edge> = edges.iter().filter(|e| e.src == src).collect();
        assert_eq!(to.len(), 1, "{src}: {edges:#?}");
        assert_eq!(conf(&edges, src, dst), high(), "{src}: {edges:#?}");
    }
}

/// Lua: a file-scope `local a = m.b` with `m` a `require` is the call
/// `m.b(…)`, resolved through the module and the table it returns
/// (`return { check = check_phase }`), at that call's confidence. The
/// direct `m.check(…)` binds the same way. Near miss: an alias of a name
/// no `require` binds (`local log = ngx.log`) stays refused.
#[test]
fn a_lua_member_alias_of_a_required_module_resolves_through_its_return_table() {
    let edges = graph(&[
        (
            "kong/pdk/private/phases.lua",
            "local function check_phase(p)\nend\nlocal function log()\nend\nreturn {\n  check = check_phase,\n  log = log,\n}\n",
        ),
        (
            "kong/pdk/client.lua",
            "local phase_checker = require \"kong.pdk.private.phases\"\nlocal check_phase = phase_checker.check\nlocal log = ngx.log\nlocal function run()\n  phase_checker.check(1)\n  check_phase(2)\n  log(3)\nend\n",
        ),
    ]);
    assert_eq!(
        conf(&edges, "run", "check_phase"),
        vec!["high".to_string(), "high".to_string()],
        "{edges:#?}"
    );
    assert!(conf(&edges, "run", "log").is_empty(), "{edges:#?}");
}

/// WS21 (gap 12): a construction is a call of the constructed type's
/// constructor. A type the tree declares once binds at high; Swift's
/// `Engine()` is matched by name onto `init` and stays medium.
#[test]
fn a_construction_reaches_the_constructor_of_its_type() {
    let edges = graph(&[
        (
            "a.cs",
            "namespace App { class P { public P() {} } class Q { void Make() { var p = new P(); P q = new(); } } }\n",
        ),
        (
            "a.rb",
            "class Coll\n  def initialize(x)\n  end\nend\ndef make_rb\n  Coll.new(1)\nend\n",
        ),
        (
            "a.php",
            "<?php\nclass Box { function __construct($x) {} }\nfunction make_php() { return new Box(1); }\n",
        ),
        (
            "a.sol",
            "contract Child { constructor(uint x) {} }\ncontract F { function make() public { Child c = new Child(1); } }\n",
        ),
        (
            "a.ps1",
            "class Crate {\n  Crate([int]$x) { }\n}\nfunction MakePs {\n  $b = [Crate]::new(1)\n}\n",
        ),
        (
            "a.m",
            "@implementation Cell\n- (instancetype)initWithX:(int)x { return self; }\n@end\nvoid make_objc(void) { Cell *c = [[Cell alloc] initWithX:1]; }\n",
        ),
        (
            "a.swift",
            "class Engine {\n  init() { }\n}\nfunc makeSwift() {\n  let e = Engine()\n}\n",
        ),
    ]);
    assert_eq!(
        conf(&edges, "App.Q.Make", "App.P.P"),
        vec!["high".to_string(), "high".to_string()],
        "{edges:#?}"
    );
    for (src, dst) in [
        ("make_rb", "Coll::initialize"),
        ("make_php", "Box::__construct"),
        ("F.make", "Child.constructor"),
        ("MakePs", "Crate.Crate"),
        ("make_objc", "Cell::initWithX"),
    ] {
        assert_eq!(conf(&edges, src, dst), high(), "{src}: {edges:#?}");
    }
    assert_eq!(
        conf(&edges, "makeSwift", "Engine.init"),
        medium(),
        "{edges:#?}"
    );
}

/// WS21 (gap 13): constructor delegation and `super` calls reach the
/// delegated-to constructor or the base's member, never the caller.
#[test]
fn constructor_delegation_reaches_the_base_or_sibling_constructor() {
    let edges = graph(&[
        (
            "Box.java",
            "class Base { Base(int x) { } }\nclass Box extends Base {\n  Box() { this(1); }\n  Box(int y) { super(y); }\n}\n",
        ),
        (
            "a.js",
            "class P { constructor(x) { this.x = x; } }\nclass K extends P { constructor() { super(1); } }\n",
        ),
        (
            "a.cs",
            "namespace N { class B { public B(int x) { } }\nclass D : B { public D() : this(1) { } public D(int y) : base(y) { } } }\n",
        ),
        (
            "a.cpp",
            "class Root { public: Root(int x) {} };\nclass Leaf : public Root { public: Leaf() : Root(1) {} Leaf(int y) : Leaf() {} };\n",
        ),
        (
            "a.rb",
            "class Animal\n  def speak\n  end\nend\nclass Dog < Animal\n  def speak\n    super\n  end\nend\n",
        ),
        (
            "a.sol",
            "contract Owned { constructor(uint x) {} }\ncontract Token is Owned { constructor(uint y) Owned(y) {} }\n",
        ),
    ]);
    for (src, dst) in [
        ("Base.Base", ""),
        ("K.constructor", "P.constructor"),
        ("N.D.D", "N.B.B"),
        ("Leaf::Leaf", "Root::Root"),
        ("Dog::speak", "Animal::speak"),
        ("Token.constructor", "Owned.constructor"),
        ("Box.Box", "Base.Base"),
    ] {
        if dst.is_empty() {
            continue;
        }
        assert!(
            conf(&edges, src, dst) == high(),
            "{src} -> {dst}: {edges:#?}"
        );
    }
    // `this(1)` / `: this(1)` / `: Leaf()` reach the sibling constructor
    // only, never the constructor they are written in.
    for own in ["Box.Box", "N.D.D", "Leaf::Leaf"] {
        let to_self: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.src == own && e.dst == own)
            .collect();
        assert_eq!(to_self.len(), 1, "{own}: {edges:#?}");
    }
}

/// WS21 near misses: a construction or `super` call whose type or base
/// is outside the tree never binds a same-named member of the tree.
#[test]
fn an_outside_base_or_type_binds_no_same_named_member() {
    let edges = graph(&[
        (
            "w.js",
            "import { Component } from 'react';\nclass W extends Component { constructor(p) { super(p); } }\nclass Other { constructor() {} }\n",
        ),
        (
            "post.rb",
            "class Post < ActiveRecord::Base\n  def save\n    super\n  end\nend\nclass Draft\n  def save\n  end\nend\n",
        ),
        (
            "q.cpp",
            "#include <vector>\nclass Q : public std::vector<int> { public: Q() : std::vector<int>(3), n_(0) {} int n_; };\nclass R { public: void n_() {} };\n",
        ),
        (
            "u.cs",
            "using System.Collections.Generic;\nnamespace App { class U { void M() { var l = new List<int>(); } } }\n",
        ),
        (
            "mine.cs",
            "namespace Mine.Collections { class List { public List() {} } }\n",
        ),
        (
            "go.php",
            "<?php\nnamespace App;\nfunction go() { throw new \\Exception(\"x\"); }\n",
        ),
        (
            "gen.rb",
            "module Likes\n  class Generator < Federated::Generator\n  end\nend\nmodule Polls\n  class Generator < Federated::Generator\n    def initialize(a)\n    end\n  end\nend\ndef like_it\n  Likes::Generator.new(1)\nend\n",
        ),
        (
            "h.cpp",
            "class Base { public: int value() { return 0; } };\nclass H : public Base { void* value; public: H() : value(nullptr) {} };\n",
        ),
        (
            "t.cs",
            "namespace T { class Tests { void Thing() { var t = new Thing(); } }\nclass Thing { public Thing() {} } }\n",
        ),
        (
            "lib.php",
            "<?php\nnamespace Lib\\Errors;\nclass Exception { function __construct($m) {} }\n",
        ),
    ]);
    for src in [
        "W.constructor",
        "Post::save",
        "Q::Q",
        "App.U.M",
        "App\\go",
        "go",
        "H::H",
        "like_it",
    ] {
        let out: Vec<&Edge> = edges.iter().filter(|e| e.src == src).collect();
        assert!(out.is_empty(), "{src}: {edges:#?}");
    }
    // `new Thing()` in a test method named `Thing` is the class's
    // constructor, never the method itself.
    let out: Vec<&str> = edges
        .iter()
        .filter(|e| e.src == "T.Tests.Thing")
        .map(|e| e.dst.as_str())
        .collect();
    assert_eq!(out, vec!["T.Thing.Thing"], "{edges:#?}");
}

/// WS22 (gap 7): a bodiless interface or abstract member is a
/// declaration. Duck-typed fan-out keeps only the implementations when
/// one survives, and a call whose only candidate is the declaration still
/// binds to it.
#[test]
fn duck_fan_out_drops_bodiless_declarations_when_an_implementation_survives() {
    let edges = graph(&[
        (
            "A.java",
            "interface Shape { double area(); }\nclass Sq implements Shape { public double area() { return 1; } }\nclass Ci implements Shape { public double area() { return 3; } }\nclass Use { void run() { this.holder.item.area(); } }\n",
        ),
        (
            "a.go",
            "package a\ntype Shape interface { Area() float64 }\ntype Sq struct{}\nfunc (s Sq) Area() float64 { return 1 }\nfunc run() { holder.item.Area() }\n",
        ),
        (
            "a.cpp",
            "struct Shape { virtual double area() = 0; };\nstruct Sq : Shape { double area() override { return 1; } };\nvoid run() { holder.item->area(); }\n",
        ),
        (
            "B.java",
            "interface Only { void lone(); }\nclass UseOnly { void go() { this.thing.other.lone(); } }\n",
        ),
    ]);
    let dsts = |src: &str| -> Vec<String> {
        let mut v: Vec<String> = edges
            .iter()
            .filter(|e| e.src == src)
            .map(|e| e.dst.clone())
            .collect();
        v.sort();
        v
    };
    assert_eq!(dsts("Use.run"), vec!["Ci.area", "Sq.area"], "{edges:#?}");
    assert_eq!(dsts("a.run"), vec!["a.Sq.Area"], "{edges:#?}");
    assert_eq!(dsts("run"), vec!["Sq::area"], "{edges:#?}");
    // Near miss: the declaration is the only candidate, so it stands.
    assert_eq!(dsts("UseOnly.go"), vec!["Only.lone"], "{edges:#?}");
}

/// WS22 (gap 8): a JS/TS class field holding a function is a method of
/// its class: `this.handler()` binds to it and its body's calls are its.
#[test]
fn a_class_field_function_is_a_method() {
    let edges = graph(&[
        (
            "a.js",
            "class Panel {\n  state = { open: false };\n  handle = () => { this.go(); };\n  go() { this.handle(); }\n}\n",
        ),
        (
            "b.ts",
            "class Comp {\n  private save = (x: number): void => { this.persist(x); };\n  persist(x: number) { this.save(x); }\n}\n",
        ),
    ]);
    assert_eq!(
        conf(&edges, "Panel.handle", "Panel.go"),
        high(),
        "{edges:#?}"
    );
    assert_eq!(
        conf(&edges, "Panel.go", "Panel.handle"),
        high(),
        "{edges:#?}"
    );
    assert_eq!(
        conf(&edges, "Comp.save", "Comp.persist"),
        high(),
        "{edges:#?}"
    );
    assert_eq!(
        conf(&edges, "Comp.persist", "Comp.save"),
        high(),
        "{edges:#?}"
    );
    // Near miss: a field holding data is no callable.
    assert!(!edges.iter().any(|e| e.dst == "Panel.state"), "{edges:#?}");
}

/// WS22 (gap 24): an Objective-C file's plain C calls reach the C
/// functions it defines, including one returning a pointer.
#[test]
fn objective_c_records_plain_c_calls() {
    let edges = graph(&[(
        "a.m",
        "static int square(int x) { return x * x; }\nstatic char *label(void) { return \"x\"; }\n@implementation Calc\n- (int)run:(int)v {\n  label();\n  self.block(v);\n  return square(v);\n}\n@end\n",
    )]);
    assert_eq!(conf(&edges, "Calc::run", "square"), high(), "{edges:#?}");
    assert_eq!(conf(&edges, "Calc::run", "label"), high(), "{edges:#?}");
    // Near miss: a call through a property is a block, not a method.
    assert!(
        !edges.iter().any(|e| e.dst.ends_with("block")),
        "{edges:#?}"
    );
}

/// WS22 (WS21 carry-over): Groovy, Scala and F# constructors are
/// callables; constructions and delegation reach them, never the
/// constructor a delegation is written in.
#[test]
fn groovy_scala_and_fsharp_constructions_reach_their_constructors() {
    let edges = graph(&[
        (
            "a.groovy",
            "class GBase {\n  GBase(int n) { }\n}\nclass GBox extends GBase {\n  GBox(int x) { super(x) }\n  GBox() { this(1) }\n}\nclass GMain {\n  static void run() { def b = new GBox(1) }\n}\n",
        ),
        (
            "a.scala",
            "class SBase(n: Int)\nclass SBox(x: Int) extends SBase(x) {\n  def this() = this(1)\n}\nclass SPlain {\n  def go(): Unit = ()\n}\nobject SMain {\n  def run(): Unit = { val b = new SBox(); val q = new SPlain() }\n}\n",
        ),
        (
            "a.fs",
            "module M\ntype FBase(n: int) =\n  member this.N = n\ntype FBox(x: int) =\n  inherit FBase(x)\n  new() = FBox(1)\n  member this.X = x\nlet make () = FBox(2)\n",
        ),
    ]);
    let lines = |src: &str, dst: &str| {
        edges
            .iter()
            .filter(|e| e.src == src && e.dst == dst)
            .count()
    };
    assert_eq!(lines("GMain.run", "GBox.GBox"), 1, "{edges:#?}");
    assert_eq!(lines("GBox.GBox", "GBase.GBase"), 1, "{edges:#?}");
    // `this(1)` reaches the one-argument constructor, not itself.
    assert_eq!(lines("GBox.GBox", "GBox.GBox"), 1, "{edges:#?}");
    assert_eq!(lines("SMain::run", "SBox::SBox"), 1, "{edges:#?}");
    assert_eq!(lines("SBox::SBox", "SBase::SBase"), 1, "{edges:#?}");
    assert_eq!(lines("SBox::SBox", "SBox::SBox"), 1, "{edges:#?}");
    // Near miss: a class with no parameter list has no constructor node.
    assert!(
        !edges.iter().any(|e| e.dst.starts_with("SPlain::SPlain")),
        "{edges:#?}"
    );
    assert_eq!(lines("FBox.FBox", "FBase.FBase"), 1, "{edges:#?}");
    assert_eq!(lines("FBox.FBox", "FBox.FBox"), 1, "{edges:#?}");
    assert_eq!(lines("make", "FBox.FBox"), 2, "{edges:#?}");
}

/// WS22: narrowing a fan-out to its concrete candidates never leaves only
/// the calling method itself, whose self-edge is not drawn: the site
/// keeps the declarations it reached before.
#[test]
fn declaration_narrowing_never_leaves_only_the_caller() {
    let edges = graph(&[
        (
            "m/Mapper.java",
            "package m;\npublic interface Mapper { void del(Long id); }\n",
        ),
        (
            "s/Service.java",
            "package s;\npublic interface Service { void del(Long id); }\n",
        ),
        (
            "s/impl/Impl.java",
            "package s.impl;\nimport m.Mapper;\nimport s.Service;\npublic class Impl implements Service {\n  private Mapper mapper;\n  public void del(Long id) { mapper.del(id); }\n}\n",
        ),
    ]);
    let mut out: Vec<&str> = edges
        .iter()
        .filter(|e| e.src == "s.impl.Impl.del")
        .map(|e| e.dst.as_str())
        .collect();
    out.sort();
    assert_eq!(out, vec!["m.Mapper.del", "s.Service.del"], "{edges:#?}");
}

/// WS22: a bare call to a member the caller's own class declares
/// abstract is a call of that declaration, never a guess among the
/// anonymous bodies of the same file that implement it.
#[test]
fn a_call_to_an_own_abstract_member_keeps_the_declaration() {
    let edges = graph(&[(
        "S.java",
        "class Sup {\n  abstract static class It {\n    void find() { boolean b = next(1); }\n    abstract boolean next(int v);\n  }\n  It a() { return new It() { boolean next(int v) { return true; } }; }\n}\n",
    )]);
    let out: Vec<(&str, &str)> = edges
        .iter()
        .filter(|e| e.src == "Sup.It.find")
        .map(|e| (e.dst.as_str(), e.confidence.as_str()))
        .collect();
    assert_eq!(out, vec![("Sup.It.next", "high")], "{edges:#?}");
}

/// WS22: a concrete candidate the call's argument count rules out is no
/// survivor either: the site keeps the declaration it reached.
#[test]
fn declaration_narrowing_ignores_survivors_the_arity_rules_out() {
    let edges = graph(&[
        (
            "m/Daos.java",
            "package m;\npublic interface Daos { Object rows(); }\n",
        ),
        (
            "api/Stats.java",
            "package api;\npublic class Stats { public Object rows(int a) { return null; } }\n",
        ),
        (
            "m/Fetchers.java",
            "package m;\npublic class Fetchers {\n  private Daos dao;\n  public Object rows() { return dao.rows(); }\n}\n",
        ),
    ]);
    let out: Vec<&str> = edges
        .iter()
        .filter(|e| e.src == "m.Fetchers.rows")
        .map(|e| e.dst.as_str())
        .collect();
    assert_eq!(out, vec!["m.Daos.rows"], "{edges:#?}");
}

/// WS22: the cap still applies to the implementations a duck-typed
/// fan-out keeps once its declarations are dropped.
#[test]
fn the_fan_out_cap_applies_after_declarations_are_dropped() {
    let mut files: Vec<(String, String)> = vec![(
        "I.java".into(),
        "interface I { void go(); }\nclass Use { void run() { this.a.b.go(); } }\n"
            .into(),
    )];
    for i in 0..6 {
        files.push((
            format!("C{i}.java"),
            format!("class C{i} implements I {{ public void go() {{}} }}\n"),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let edges = graph(&refs);
    assert!(!edges.iter().any(|e| e.src == "Use.run"), "{edges:#?}");
}

/// WS22: a Java variable named like the last segment of an imported
/// package (`import g.preferences.GuiPreferences;` then
/// `preferences.getAi()`) is no call through that package: the lone
/// implementation in another package is a guess, never `high`.
#[test]
fn a_variable_named_like_an_imported_package_is_no_module_alias() {
    let edges = graph(&[
        (
            "l/preferences/CliPreferences.java",
            "package l.preferences;\npublic interface CliPreferences { String getAi(); }\n",
        ),
        (
            "l/preferences/JabRefCliPreferences.java",
            "package l.preferences;\npublic class JabRefCliPreferences implements CliPreferences { public String getAi() { return \"\"; } }\n",
        ),
        (
            "g/preferences/GuiPreferences.java",
            "package g.preferences;\nimport l.preferences.CliPreferences;\npublic interface GuiPreferences extends CliPreferences { }\n",
        ),
        (
            "g/ui/View.java",
            "package g.ui;\nimport g.preferences.GuiPreferences;\npublic class View { private GuiPreferences preferences; void init() { preferences.getAi(); } }\n",
        ),
    ]);
    assert_eq!(
        conf(
            &edges,
            "g.ui.View.init",
            "l.preferences.JabRefCliPreferences.getAi"
        ),
        medium(),
        "{edges:#?}"
    );
    // Nor when the imported package is the implementation's own.
    let edges = graph(&[
        (
            "l/preferences/CliPreferences.java",
            "package l.preferences;\npublic interface CliPreferences { String getAi(); }\n",
        ),
        (
            "l/preferences/JabRefCliPreferences.java",
            "package l.preferences;\npublic class JabRefCliPreferences implements CliPreferences { public String getAi() { return \"\"; } }\n",
        ),
        (
            "g/ui/View.java",
            "package g.ui;\nimport l.preferences.CliPreferences;\npublic class View { private CliPreferences preferences; void init() { preferences.getAi(); } }\n",
        ),
    ]);
    assert!(
        conf(
            &edges,
            "g.ui.View.init",
            "l.preferences.JabRefCliPreferences.getAi"
        )
        .iter()
        .all(|c| c != "high"),
        "{edges:#?}"
    );
    // Near miss: importing a nested type (`a.Tracker.Stat`) aliases its
    // class, and a static call through that class is its member: `high`.
    let edges = graph(&[
        (
            "a/Tracker.java",
            "package a;\npublic class Tracker {\n  public enum Stat { X }\n  public static void track(Stat s) {}\n  public static void track(Stat s, String p) {}\n}\n",
        ),
        (
            "b/U.java",
            "package b;\nimport a.Tracker;\nimport a.Tracker.Stat;\npublic class U { void m() { Tracker.track(Stat.X); } }\n",
        ),
    ]);
    assert_eq!(
        conf(&edges, "b.U.m", "a.Tracker.track"),
        high(),
        "{edges:#?}"
    );
}
