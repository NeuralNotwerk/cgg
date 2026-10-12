import os, sys
ROOT = sys.argv[1]
NAMES = ["Alpha","Beta","Gamma","Delta","Epsilon","Zeta"]
def w(d, f, s):
    p = os.path.join(d, f); os.makedirs(os.path.dirname(p), exist_ok=True); open(p, "w").write(s)
# mode: A = typed param (2 impls), Af = typed field (2 impls), Bp = typed param (6), B = weak/untyped/generic (6)
def D(lang, v, mode): return os.path.join(ROOT, f"{lang}-{v}-{mode}")
def pick(mode, param, field, weak):
    return {"A": param, "Bp": param, "Af": field, "Bf": field, "B": weak}[mode]

def gen_java(mode, n, abstract):
    d = D("java", "abs" if abstract else "iface", mode)
    if abstract:
        w(d, "Processor.java", "public abstract class Processor {\n    public abstract void process();\n    public abstract void unusedOp();\n}\n"); rel = "extends"
    else:
        w(d, "Processor.java", "public interface Processor {\n    void process();\n    void unusedOp();\n}\n"); rel = "implements"
    for nm in NAMES[:n]:
        w(d, f"{nm}.java", f"public class {nm} {rel} Processor {{\n    @Override\n    public void process() {{ System.out.println(\"{nm}\"); }}\n    @Override\n    public void unusedOp() {{ System.out.println(\"u\"); }}\n}}\n")
    news = "\n".join(f"        run(new {nm}());" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("    static void run(Processor p) {\n        p.process();\n    }\n", news),
        ("    private Processor field;\n    Main(Processor p) { this.field = p; }\n    void runField() { field.process(); }\n", "        new Main(new Alpha()).runField();\n        new Main(new Beta()).runField();"),
        ("    static <T extends Processor> void run(T p) {\n        p.process();\n    }\n", news))
    w(d, "Main.java", f"public class Main {{\n{caller}    public static void main(String[] args) {{\n{main}\n    }}\n}}\n")

def gen_csharp(mode, n, abstract):
    d = D("csharp", "abs" if abstract else "iface", mode)
    if abstract:
        w(d, "Processor.cs", "public abstract class Processor {\n    public abstract void Process();\n    public abstract void UnusedOp();\n}\n"); ov = "public override"; base = "Processor"
    else:
        w(d, "Processor.cs", "public interface IProcessor {\n    void Process();\n    void UnusedOp();\n}\n"); ov = "public"; base = "IProcessor"
    for nm in NAMES[:n]:
        w(d, f"{nm}.cs", f"public class {nm} : {base} {{\n    {ov} void Process() {{ System.Console.WriteLine(\"{nm}\"); }}\n    {ov} void UnusedOp() {{ System.Console.WriteLine(\"u\"); }}\n}}\n")
    news = "\n".join(f"        Run(new {nm}());" for nm in NAMES[:n])
    caller, main = pick(mode,
        (f"    static void Run({base} p) {{\n        p.Process();\n    }}\n", news),
        (f"    private {base} field;\n    Program({base} p) {{ field = p; }}\n    void RunField() {{ field.Process(); }}\n", "        new Program(new Alpha()).RunField();\n        new Program(new Beta()).RunField();"),
        (f"    static void Run<T>(T p) where T : {base} {{\n        p.Process();\n    }}\n", news))
    w(d, "Program.cs", f"public class Program {{\n{caller}    public static void Main(string[] args) {{\n{main}\n    }}\n}}\n")

def gen_go(mode, n):
    d = D("go", "iface", mode)
    w(d, "go.mod", "module example.com/fx\n\ngo 1.22\n")
    w(d, "processor.go", "package main\n\ntype processor interface {\n\tprocess()\n\tunusedOp()\n}\n")
    for nm in NAMES[:n]:
        l = nm.lower()
        w(d, f"{l}.go", f"package main\n\nimport \"fmt\"\n\ntype {l} struct{{}}\n\nfunc (x {l}) process() {{ fmt.Println(\"{nm}\") }}\n\nfunc (x {l}) unusedOp() {{ fmt.Println(\"u\") }}\n")
    news = "\n".join(f"\trun({nm.lower()}{{}})" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("func run(p processor) {\n\tp.process()\n}\n", news),
        ("type holder struct {\n\tfield processor\n}\n\nfunc (h holder) runField() {\n\th.field.process()\n}\n", "\tholder{field: alpha{}}.runField()\n\tholder{field: beta{}}.runField()"),
        ("func run[T processor](p T) {\n\tp.process()\n}\n", news))
    w(d, "main.go", f"package main\n\n{caller}\nfunc main() {{\n{main}\n}}\n")

def gen_ts(mode, n, abstract):
    d = D("typescript", "abs" if abstract else "iface", mode)
    if abstract:
        w(d, "processor.ts", "export abstract class Processor {\n  abstract process(): void;\n  abstract unusedOp(): void;\n}\n"); rel = "extends"
    else:
        w(d, "processor.ts", "export interface Processor {\n  process(): void;\n  unusedOp(): void;\n}\n"); rel = "implements"
    for nm in NAMES[:n]:
        w(d, f"{nm.lower()}.ts", f"import {{ Processor }} from \"./processor\";\n\nexport class {nm} {rel} Processor {{\n  process(): void {{ console.log(\"{nm}\"); }}\n  unusedOp(): void {{ console.log(\"u\"); }}\n}}\n")
    imps = "\n".join(f"import {{ {nm} }} from \"./{nm.lower()}\";" for nm in NAMES[:n])
    news = "\n".join(f"  run(new {nm}());" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("function run(p: Processor): void {\n  p.process();\n}\n", news),
        ("class Holder {\n  constructor(private field: Processor) {}\n  runField(): void { this.field.process(); }\n}\n", "  new Holder(new Alpha()).runField();\n  new Holder(new Beta()).runField();"),
        ("function run(p: any): void {\n  p.process();\n}\n", news))
    w(d, "main.ts", f"import {{ Processor }} from \"./processor\";\n{imps}\n\n{caller}\nexport function main(): void {{\n{main}\n}}\n\nmain();\n")

def gen_kotlin(mode, n):
    d = D("kotlin", "iface", mode)
    w(d, "Processor.kt", "interface Processor {\n    fun process()\n    fun unusedOp()\n}\n")
    for nm in NAMES[:n]:
        w(d, f"{nm}.kt", f"class {nm} : Processor {{\n    override fun process() {{ println(\"{nm}\") }}\n    override fun unusedOp() {{ println(\"u\") }}\n}}\n")
    news = "\n".join(f"    run({nm}())" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("fun run(p: Processor) {\n    p.process()\n}\n", news),
        ("class Holder(private val field: Processor) {\n    fun runField() { field.process() }\n}\n", "    Holder(Alpha()).runField()\n    Holder(Beta()).runField()"),
        ("fun <T : Processor> run(p: T) {\n    p.process()\n}\n", news))
    w(d, "Main.kt", f"{caller}\nfun main() {{\n{main}\n}}\n")

def gen_swift(mode, n):
    d = D("swift", "proto", mode)
    w(d, "Processor.swift", "protocol Processor {\n    func process()\n    func unusedOp()\n}\n")
    for nm in NAMES[:n]:
        w(d, f"{nm}.swift", f"struct {nm}: Processor {{\n    func process() {{ print(\"{nm}\") }}\n    func unusedOp() {{ print(\"u\") }}\n}}\n")
    news = "\n".join(f"    run({nm}())" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("func run(_ p: Processor) {\n    p.process()\n}\n", news),
        ("struct Holder {\n    let field: Processor\n    func runField() { field.process() }\n}\n", "    Holder(field: Alpha()).runField()\n    Holder(field: Beta()).runField()"),
        ("func run<T: Processor>(_ p: T) {\n    p.process()\n}\n", news))
    w(d, "main.swift", f"{caller}\nfunc main() {{\n{main}\n}}\n\nmain()\n")

def gen_scala(mode, n):
    d = D("scala", "trait", mode)
    w(d, "Processor.scala", "trait Processor {\n  def process(): Unit\n  def unusedOp(): Unit\n}\n")
    for nm in NAMES[:n]:
        w(d, f"{nm}.scala", f"class {nm} extends Processor {{\n  def process(): Unit = println(\"{nm}\")\n  def unusedOp(): Unit = println(\"u\")\n}}\n")
    news = "\n".join(f"    run(new {nm}())" for nm in NAMES[:n])
    if mode in ("Af", "Bf"):
        w(d, "Holder.scala", "class Holder(field: Processor) {\n  def runField(): Unit = field.process()\n}\n")
    caller, main = pick(mode,
        ("  def run(p: Processor): Unit = {\n    p.process()\n  }\n", news),
        ("", "    new Holder(new Alpha()).runField()\n    new Holder(new Beta()).runField()"),
        ("  def run[T <: Processor](p: T): Unit = {\n    p.process()\n  }\n", news))
    w(d, "Main.scala", f"object Main {{\n{caller}\n  def main(args: Array[String]): Unit = {{\n{main}\n  }}\n}}\n")

def gen_php(mode, n):
    d = D("php", "iface", mode)
    w(d, "Processor.php", "<?php\ninterface Processor {\n    public function process();\n    public function unusedOp();\n}\n")
    for nm in NAMES[:n]:
        w(d, f"{nm}.php", f"<?php\nrequire_once 'Processor.php';\nclass {nm} implements Processor {{\n    public function process() {{ echo \"{nm}\"; }}\n    public function unusedOp() {{ echo \"u\"; }}\n}}\n")
    reqs = "\n".join(f"require_once '{nm}.php';" for nm in NAMES[:n])
    news = "\n".join(f"    run(new {nm}());" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("function run(Processor $p) {\n    $p->process();\n}\n", news),
        ("class Holder {\n    private Processor $field;\n    public function __construct(Processor $p) { $this->field = $p; }\n    public function runField() { $this->field->process(); }\n}\n", "    (new Holder(new Alpha()))->runField();\n    (new Holder(new Beta()))->runField();"),
        ("function run($p) {\n    $p->process();\n}\n", news))
    w(d, "main.php", f"<?php\n{reqs}\n\n{caller}\nfunction main() {{\n{main}\n}}\n\nmain();\n")

def gen_python(mode, n, proto):
    d = D("python", "proto" if proto else "abc", mode)
    if proto:
        w(d, "processor.py", "from typing import Protocol\n\n\nclass Processor(Protocol):\n    def process(self) -> None: ...\n\n    def unused_op(self) -> None: ...\n"); base = ""
    else:
        w(d, "processor.py", "from abc import ABC, abstractmethod\n\n\nclass Processor(ABC):\n    @abstractmethod\n    def process(self) -> None: ...\n\n    @abstractmethod\n    def unused_op(self) -> None: ...\n"); base = "(Processor)"
    for nm in NAMES[:n]:
        imp = "" if proto else "from processor import Processor\n\n\n"
        w(d, f"{nm.lower()}.py", f"{imp}class {nm}{base}:\n    def process(self) -> None:\n        print(\"{nm}\")\n\n    def unused_op(self) -> None:\n        print(\"u\")\n")
    imps = "\n".join(f"from {nm.lower()} import {nm}" for nm in NAMES[:n])
    news = "\n".join(f"    run({nm}())" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("def run(p: Processor) -> None:\n    p.process()\n", news),
        ("class Holder:\n    def __init__(self, field: Processor) -> None:\n        self.field = field\n\n    def run_field(self) -> None:\n        self.field.process()\n", "    Holder(Alpha()).run_field()\n    Holder(Beta()).run_field()"),
        ("def run(p):\n    p.process()\n", news))
    w(d, "main.py", f"from processor import Processor\n{imps}\n\n\n{caller}\n\ndef main() -> None:\n{main}\n\n\nif __name__ == \"__main__\":\n    main()\n")

def gen_cpp(mode, n):
    d = D("cpp", "virt", mode)
    w(d, "processor.h", "#pragma once\nstruct Processor {\n    virtual ~Processor() = default;\n    virtual void process() = 0;\n    virtual void unusedOp() = 0;\n};\n")
    for nm in NAMES[:n]:
        l = nm.lower()
        w(d, f"{l}.h", f"#pragma once\n#include <cstdio>\n#include \"processor.h\"\nstruct {nm} : Processor {{\n    void process() override;\n    void unusedOp() override;\n}};\n")
        w(d, f"{l}.cpp", f"#include \"{l}.h\"\nvoid {nm}::process() {{ std::puts(\"{nm}\"); }}\nvoid {nm}::unusedOp() {{ std::puts(\"u\"); }}\n")
    incs = "\n".join(f"#include \"{nm.lower()}.h\"" for nm in NAMES[:n])
    objs = "\n".join(f"    {nm} o{i};" for i, nm in enumerate(NAMES[:n]))
    news = objs + "\n" + "\n".join(f"    run(o{i});" for i in range(n))
    caller, main = pick(mode,
        ("void run(Processor& p) {\n    p.process();\n}\n", news),
        ("struct Holder {\n    Processor* field;\n    void runField() { field->process(); }\n};\n", objs + "\n    Holder h0{&o0};\n    h0.runField();\n    Holder h1{&o1};\n    h1.runField();"),
        ("template <typename T>\nvoid run(T& p) {\n    p.process();\n}\n", news))
    w(d, "main.cpp", f"#include \"processor.h\"\n{incs}\n\n{caller}\nint main() {{\n{main}\n    return 0;\n}}\n")

def gen_dart(mode, n):
    d = D("dart", "abs", mode)
    w(d, "processor.dart", "abstract class Processor {\n  void process();\n  void unusedOp();\n}\n")
    for nm in NAMES[:n]:
        w(d, f"{nm.lower()}.dart", f"import 'processor.dart';\n\nclass {nm} implements Processor {{\n  @override\n  void process() {{\n    print('{nm}');\n  }}\n\n  @override\n  void unusedOp() {{\n    print('u');\n  }}\n}}\n")
    imps = "\n".join(f"import '{nm.lower()}.dart';" for nm in NAMES[:n])
    news = "\n".join(f"  run({nm}());" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("void run(Processor p) {\n  p.process();\n}\n", news),
        ("class Holder {\n  final Processor field;\n  Holder(this.field);\n  void runField() {\n    field.process();\n  }\n}\n", "  Holder(Alpha()).runField();\n  Holder(Beta()).runField();"),
        ("void run(dynamic p) {\n  p.process();\n}\n", news))
    w(d, "main.dart", f"import 'processor.dart';\n{imps}\n\n{caller}\nvoid main() {{\n{main}\n}}\n")

def gen_rust(mode, n):
    d = D("rust", "trait", mode)
    impls = "".join(f"pub struct {nm};\nimpl Processor for {nm} {{\n    fn process(&self) {{ println!(\"{nm}\"); }}\n    fn unused_op(&self) {{ println!(\"u\"); }}\n}}\n\n" for nm in NAMES[:n])
    news = "\n".join(f"    run(&{nm});" for nm in NAMES[:n])
    caller, main = pick(mode,
        ("fn run(p: &dyn Processor) {\n    p.process();\n}\n", news),
        ("struct Holder {\n    field: Box<dyn Processor>,\n}\nimpl Holder {\n    fn run_field(&self) {\n        self.field.process();\n    }\n}\n", "    Holder { field: Box::new(Alpha) }.run_field();\n    Holder { field: Box::new(Beta) }.run_field();"),
        ("fn run<T: Processor>(p: &T) {\n    p.process();\n}\n", news))
    w(d, "src/main.rs", f"pub trait Processor {{\n    fn process(&self);\n    fn unused_op(&self);\n}}\n\n{impls}{caller}\nfn main() {{\n{main}\n}}\n")
    w(d, "Cargo.toml", "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")

for mode, n in (("Bf", 6),):
    gen_java(mode, n, False); gen_java(mode, n, True)
    gen_csharp(mode, n, False); gen_csharp(mode, n, True)
    gen_go(mode, n); gen_ts(mode, n, False); gen_ts(mode, n, True)
    gen_kotlin(mode, n); gen_swift(mode, n); gen_scala(mode, n); gen_php(mode, n)
    gen_python(mode, n, False); gen_python(mode, n, True)
    gen_cpp(mode, n); gen_dart(mode, n); gen_rust(mode, n)
