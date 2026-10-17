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
