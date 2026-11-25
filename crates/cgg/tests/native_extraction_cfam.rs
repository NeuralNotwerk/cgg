//! C/C++ extraction of macro-shaped code the grammar cannot read on its
//! own: class-body macro invocations, export macros in a class head,
//! calling-convention macros, and function-defining macros from outside
//! the tree (0.14.0 framework audit, C and C++ prerequisites).

use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

fn graph(dir: &Path) -> Value {
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .args(["-t", "json", "--jobs", "1"])
        .arg(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

/// `(qualified name, attributes)` of every callable, sorted.
fn callables(g: &Value) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = g["callables"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| {
            let attrs = c["attributes"]
                .as_array()
                .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
                .unwrap_or_default();
            (c["qualified_name"].as_str().unwrap().to_string(), attrs)
        })
        .collect();
    v.sort();
    v
}

fn names(g: &Value) -> Vec<String> {
    callables(g).into_iter().map(|(n, _)| n).collect()
}

fn attrs_of(g: &Value, qn: &str) -> Vec<String> {
    callables(g)
        .into_iter()
        .find(|(n, _)| n == qn)
        .map(|(_, a)| a)
        .unwrap_or_else(|| panic!("no callable {qn} in {:?}", names(g)))
}

/// Whether an edge runs from the callable named `src` to the one named `dst`.
fn has_edge(g: &Value, src: &str, dst: &str) -> bool {
    let id = |qn: &str| {
        g["callables"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, c)| c["qualified_name"] == qn)
            .map(|(k, _)| k.clone())
    };
    let (Some(s), Some(d)) = (id(src), id(dst)) else {
        return false;
    };
    g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["src"] == s.as_str() && e["dst"] == d.as_str())
}

#[test]
fn class_body_macros_are_member_annotations_not_methods() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "actor.h",
        "#include \"CoreMinimal.h\"\nUCLASS()\nclass AMyActor : public AActor {\n  GENERATED_BODY()\npublic:\n  virtual void BeginPlay() override;\n  UFUNCTION(BlueprintCallable, Category = \"Bomb\")\n  void Fire();\n  UFUNCTION()\n  void OnRep_Health();\n  Q_INVOKABLE void go();\n  TF_DISALLOW_COPY_AND_ASSIGN(AMyActor);\n  METHOD_ADD(AMyActor::Fire, \"/fire\", Get);\n  virtual ~AMyActor();\n};\n",
    );
    write(
        tmp.path(),
        "actor.cpp",
        "#include \"actor.h\"\nvoid AMyActor::BeginPlay() {}\nvoid AMyActor::Fire() {}\nvoid AMyActor::OnRep_Health() {}\nvoid AMyActor::go() {}\n",
    );
    let g = graph(tmp.path());
    let n = names(&g);
    for macro_name in ["GENERATED_BODY", "UFUNCTION", "TF_DISALLOW", "METHOD_ADD"] {
        assert!(!n.iter().any(|x| x.contains(macro_name)), "{n:?}");
    }
    // The annotation rides on the out-of-line body the header declares.
    assert!(
        attrs_of(&g, "AMyActor::Fire")
            .contains(&"UFUNCTION(BlueprintCallable, Category = \"Bomb\")".to_string())
    );
    assert!(attrs_of(&g, "AMyActor::OnRep_Health").contains(&"UFUNCTION()".to_string()));
    assert!(attrs_of(&g, "AMyActor::go").contains(&"Q_INVOKABLE".to_string()));
    assert!(
        !attrs_of(&g, "AMyActor::BeginPlay")
            .iter()
            .any(|a| a.contains("UFUNCTION"))
    );
}

#[test]
fn a_member_after_a_semicolon_less_macro_line_is_kept() {
    // `Q_OBJECT` / `GENERATED_BODY()` with no `;` used to swallow the
    // member after it.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "t.cpp",
        "#include <QtTest>\nclass TestGroup : public QObject {\n  Q_OBJECT\npublic:\n  void initTestCase() {}\n  void testAdd() {}\n};\nclass U : public UObject {\n  GENERATED_BODY()\n  void first() {}\n  void second() {}\n};\nUCLASS(BlueprintType)\nclass V : public UObject {\n\tGENERATED_BODY()\n\npublic:\n\tUFUNCTION(BlueprintPure)\n\tstatic bool alive() { return true; }\n};\n",
    );
    let n = names(&graph(tmp.path()));
    for want in [
        "TestGroup::initTestCase",
        "TestGroup::testAdd",
        "U::first",
        "U::second",
        "V::alive",
    ] {
        assert!(n.contains(&want.to_string()), "{want} missing from {n:?}");
    }
    assert!(!n.iter().any(|x| x.contains("GENERATED_BODY")), "{n:?}");
}

#[test]
fn an_export_macro_in_a_class_head_keeps_the_name_and_bases() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "comp.h",
        "class UActorComponent {\npublic:\n  void Register();\n};\nclass BOMBER_API UMapComp final : public UActorComponent\n{\npublic:\n  virtual void BeginPlay() override;\n};\nclass Q_CORE_EXPORT QThing : public UActorComponent {\npublic:\n  void run();\n};\n",
    );
    write(
        tmp.path(),
        "comp.cpp",
        "#include \"comp.h\"\nvoid UActorComponent::Register() {}\nvoid UMapComp::BeginPlay() { }\nvoid QThing::run() { }\nvoid use() { UMapComp m; m.Register(); QThing q; q.Register(); }\n",
    );
    let g = graph(tmp.path());
    let a = attrs_of(&g, "UMapComp::BeginPlay");
    assert!(a.contains(&"override".to_string()), "{a:?}");
    assert!(names(&g).contains(&"QThing::run".to_string()));
    // The inherited member is found through the base list, so the typed
    // calls bind with certainty (a by-name guess was `medium`).
    let site: Vec<&Value> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["site_line"] == 5)
        .collect();
    assert!(has_edge(&g, "use", "UActorComponent::Register"));
    assert_eq!(site.len(), 2, "both typed calls reach the inherited member");
    assert!(site.iter().all(|e| e["confidence"] == "high"), "{site:?}");
}

#[test]
fn a_calling_convention_macro_does_not_hide_the_function() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "plugin.cpp",
        "AudioProcessor* JUCE_CALLTYPE createPluginFilter() {\n    return new MyProc();\n}\nLRESULT CALLBACK WndProc(HWND h, UINT m) { return 0; }\n",
    );
    write(
        tmp.path(),
        "win.c",
        "static void helper(void) {}\nHANDLE WINAPI open_it(void) { helper(); return 0; }\n",
    );
    let g = graph(tmp.path());
    let n = names(&g);
    for want in ["createPluginFilter", "WndProc", "open_it"] {
        assert!(n.contains(&want.to_string()), "{want} missing from {n:?}");
    }
    assert!(has_edge(&g, "open_it", "helper"));
}

#[test]
fn function_defining_macros_name_the_function_they_define() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ext.c",
        "#include \"php.h\"\nstatic int util(int a){ return a; }\nPHP_METHOD(Redis, get){\n  zend_string *key;\n  ZEND_PARSE_PARAMETERS_START(1, 1)\n    Z_PARAM_STR(key)\n  ZEND_PARSE_PARAMETERS_END_EX(RETURN_FALSE);\n  util(1);\n}\nZEND_METHOD(Redis, set){ util(2); }\n",
    );
    write(
        tmp.path(),
        "prog.bpf.c",
        "static int count(void){ return 1; }\nSEC(\"kprobe/do_unlinkat\")\nint BPF_KPROBE(do_unlinkat, int dfd){ return count(); }\nint BPF_PROG(fexit_skb, struct sk_buff *skb){ return 0; }\n",
    );
    write(
        tmp.path(),
        "test/t.c",
        "int mul(int a,int b){ return a*b; }\nZTEST(suite_a, test_one){ mul(1,2); }\nTest(math, mul_works){ mul(2,3); }\nTEST(Dbl, Doubles){ mul(3,4); }\nTEST_CASE(\"adds things\", \"[math]\"){ mul(4,5); }\n",
    );
    write(
        tmp.path(),
        "catch.cpp",
        "int twice(int a){ return 2*a; }\nTEST_CASE(\"doubles\") { twice(2); }\nTEST(Gt, Case) { twice(3); }\nTEST_CASE_TEMPLATE(\"templ\", T, int, long) { twice(4); }\nTEST_CASE_FIXTURE(Fix, \"fix\") { twice(5); }\n",
    );
    let g = graph(tmp.path());
    let n = names(&g);
    for bogus in [
        "PHP_METHOD",
        "ZEND_METHOD",
        "BPF_KPROBE",
        "BPF_PROG",
        "ZTEST",
        "Test",
    ] {
        assert!(!n.contains(&bogus.to_string()), "{bogus} in {n:?}");
    }
    // Parameter-parsing macros run together in a body are not prototypes.
    assert!(
        !n.iter()
            .any(|x| x.starts_with("Z_PARAM") || x.starts_with("ZEND_PARSE")),
        "{n:?}"
    );
    assert_eq!(attrs_of(&g, "Redis::get"), vec!["PHP_METHOD(Redis, get)"]);
    assert!(has_edge(&g, "Redis::get", "util"));
    assert!(has_edge(&g, "Redis::set", "util"));
    assert!(has_edge(&g, "do_unlinkat", "count"));
    assert!(n.contains(&"fexit_skb".to_string()));
    assert!(has_edge(&g, "suite_a::test_one", "mul"));
    assert!(has_edge(&g, "math::mul_works", "mul"));
    assert!(has_edge(&g, "Dbl::Doubles", "mul"));
    assert!(has_edge(&g, "TEST_CASE_at_5", "mul"));
    assert_eq!(
        attrs_of(&g, "TEST_CASE_at_5"),
        vec!["TEST_CASE(\"adds things\", \"[math]\")"]
    );
    // C++: Catch2's TEST_CASE too; gtest's TEST keeps the macro's name.
    assert!(has_edge(&g, "TEST_CASE_at_2", "twice"));
    assert!(has_edge(&g, "TEST", "twice"));
    // doctest's template and fixture cases too: before, no callable held
    // their bodies.
    assert!(has_edge(&g, "TEST_CASE_TEMPLATE_at_4", "twice"));
    assert!(has_edge(&g, "TEST_CASE_FIXTURE_at_5", "twice"));
}
