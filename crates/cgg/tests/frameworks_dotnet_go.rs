//! Framework rows for .NET (C# and its F# twins) and Go added for 0.14.0.
//!
//! One fixture per row: each names the entries the row must produce and,
//! where the row has an obvious near miss (a lowercase Go `init`, a helper
//! taking an `ILambdaContext` first, an unrelated `*Base` class), checks
//! that the near miss is not entered. The count in the coverage line pins
//! both halves at once.

use std::fs;
use std::path::Path;

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

/// Run cgg over `dir`; return (mermaid graph, stderr).
fn run(dir: &Path) -> (String, String) {
    let out = dir.join("g.mmd");
    let assert = Command::cargo_bin("cgg")
        .expect("cgg binary")
        .args(["--jobs", "1", "-o"])
        .arg(&out)
        .arg(dir)
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

struct Case {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// `id (kind, N entr` as the coverage line prints it.
    coverage: &'static str,
    /// Substrings the graph must contain (entry node names).
    present: &'static [&'static str],
    /// Substrings the graph must not contain.
    absent: &'static [&'static str],
}

const CASES: &[Case] = &[
    // ---------------------------------------------------------------- C#
    Case {
        name: "harmony/csharp",
        files: &[(
            "P.cs",
            "using HarmonyLib;\n[HarmonyPatch(typeof(Foo), \"Bar\")]\nstatic class P {\n  static void Prefix() {}\n  [HarmonyPostfix] static void After() {}\n  public static void Cleanup(bool sync) {}\n}\n",
        )],
        coverage: "harmony (lifecycle, 2 entries)",
        present: &["harmony::P.Prefix", "harmony::P.After"],
        absent: &["harmony::P.Cleanup"],
    },
    Case {
        name: "harmony/fsharp",
        files: &[(
            "P.fs",
            "module Patches\nopen HarmonyLib\ntype P() =\n    static member Prefix() = ()\n    static member Helper() = ()\n",
        )],
        coverage: "harmony (lifecycle, 1 entry)",
        present: &["harmony::P.Prefix"],
        absent: &["harmony::P.Helper"],
    },
    Case {
        name: "reqnroll/csharp",
        files: &[(
            "Steps.cs",
            "using Reqnroll;\n[Binding]\npublic class Steps {\n  [Given(\"a user\")] public void GivenAUser() { Helper(); }\n  [BeforeScenario] public void Setup() {}\n  public void Helper() {}\n}\n",
        )],
        coverage: "reqnroll (test, 2 entries)",
        present: &["GivenAUser", "Steps.Setup"],
        absent: &[],
    },
    Case {
        name: "reqnroll/csharp-specflow",
        files: &[(
            "Steps.cs",
            "using TechTalk.SpecFlow;\n[Binding]\npublic class Steps {\n  [When(\"it runs\")] public void WhenItRuns() {}\n}\n",
        )],
        coverage: "reqnroll (test, 1 entry)",
        present: &["WhenItRuns"],
        absent: &[],
    },
    Case {
        name: "mcmaster-cli/csharp",
        files: &[(
            "Cmd.cs",
            "using McMaster.Extensions.CommandLineUtils;\n[Command(\"x\")]\nclass Cmd {\n  private int OnExecute() { Run(); return 0; }\n  private void Run() {}\n}\n",
        )],
        coverage: "mcmaster-cli (cli, 1 entry)",
        present: &["mcmaster-cli::Cmd.OnExecute"],
        absent: &["mcmaster-cli::Cmd.Run"],
    },
    Case {
        name: "spectre-cli/csharp",
        files: &[(
            "Hello.cs",
            "using Spectre.Console.Cli;\npublic sealed class Hello : Command<Hello.Settings> {\n  public sealed class Settings : CommandSettings {}\n  public override int Execute(CommandContext c, Settings s) { Helper(); return 0; }\n  private void Helper() {}\n}\n",
        )],
        coverage: "spectre-cli (cli, 1 entry)",
        present: &["spectre-cli::Hello.Execute"],
        absent: &["spectre-cli::Hello.Helper"],
    },
    Case {
        name: "roslyn/csharp-global-using",
        files: &[
            (
                "GlobalUsings.cs",
                "global using global::Microsoft.CodeAnalysis;\n",
            ),
            (
                "Gen.cs",
                "[Generator]\npublic sealed class Gen : IIncrementalGenerator {\n  public void Initialize(IncrementalGeneratorInitializationContext c) { Helper(); }\n  private void Helper() {}\n}\n",
            ),
        ],
        coverage: "roslyn (lifecycle, 1 entry)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "fluentvalidation/csharp",
        files: &[(
            "V.cs",
            "using FluentValidation;\npublic class OrderValidator : AbstractValidator<Order> {\n  public OrderValidator() { RuleFor(x => x.Id).Must(BeValid); }\n  private bool BeValid(int id) => id > 0;\n}\n",
        )],
        // node: false — rooted, no graph node.
        coverage: "fluentvalidation (lifecycle, 1 entry)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "fluentvalidation/fsharp",
        files: &[(
            "V.fs",
            "module Validators\nopen FluentValidation\ntype OrderValidator() =\n    inherit AbstractValidator<Order>()\n    member x.Helper() = 1\n",
        )],
        coverage: "fluentvalidation (lifecycle, 1 entry)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "fluentmigrator/csharp",
        files: &[(
            "M.cs",
            "using FluentMigrator;\n[Migration(1)]\npublic class AddUsers : Migration {\n  public override void Up() { Helper(); }\n  public override void Down() {}\n  private void Helper() {}\n}\n",
        )],
        coverage: "fluentmigrator (lifecycle, 2 entries)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "rebus/csharp",
        files: &[(
            "H.cs",
            "using Rebus.Handlers;\npublic class PingHandler : IHandleMessages<Ping> {\n  public System.Threading.Tasks.Task Handle(Ping m) { X(); return null; }\n  private void X() {}\n}\n",
        )],
        coverage: "rebus (queue, 1 entry)",
        present: &["rebus::PingHandler.Handle"],
        absent: &["rebus::PingHandler.X"],
    },
    Case {
        name: "rebus/fsharp-interface",
        files: &[(
            "H.fs",
            "module Handlers\nopen Rebus.Handlers\ntype PingHandler() =\n    interface IHandleMessages<Ping> with\n        member this.Handle(m) = Task.CompletedTask\n",
        )],
        coverage: "rebus (queue, 1 entry)",
        present: &["rebus::PingHandler.Handle"],
        absent: &[],
    },
    Case {
        name: "nservicebus/csharp",
        files: &[(
            "S.cs",
            "using NServiceBus;\npublic class OrderSaga : Saga<D>, IAmStartedByMessages<Start>, IHandleTimeouts<Late> {\n  public Task Handle(Start m, IMessageHandlerContext c) => null;\n  public Task Timeout(Late t, IMessageHandlerContext c) => null;\n  private void Helper() {}\n}\n",
        )],
        coverage: "nservicebus (queue, 2 entries)",
        present: &[
            "nservicebus::OrderSaga.Handle",
            "nservicebus::OrderSaga.Timeout",
        ],
        absent: &["nservicebus::OrderSaga.Helper"],
    },
    Case {
        name: "dotnetcore-cap/csharp",
        files: &[(
            "S.cs",
            "using DotNetCore.CAP;\npublic class Sub : ICapSubscribe {\n  [CapSubscribe(\"orders.created\")] public void OnCreated(Order o) {}\n  public void Other() {}\n}\n",
        )],
        coverage: "dotnetcore-cap (queue, 1 entry)",
        present: &["dotnetcore-cap::"],
        absent: &["Sub.Other ⟨"],
    },
    Case {
        name: "fastendpoints/csharp",
        files: &[(
            "E.cs",
            "using FastEndpoints;\npublic class GetUser : Endpoint<Req, Res> {\n  public override void Configure() { Get(\"/users/{id}\"); }\n  public override Task HandleAsync(Req r, CancellationToken c) { H(); return null; }\n  private void H() {}\n}\n",
        )],
        coverage: "fastendpoints (network, 2 entries)",
        present: &[
            "fastendpoints::GetUser.Configure",
            "fastendpoints::GetUser.HandleAsync",
        ],
        absent: &["fastendpoints::GetUser.H ⟨"],
    },
    Case {
        name: "abp-appservice/csharp",
        files: &[(
            "Books.cs",
            "using Volo.Abp.Application.Services;\npublic class BookAppService : ApplicationService {\n  public Task<int> GetCountAsync() => null;\n}\n",
        )],
        coverage: "abp-appservice (network, 1 entry)",
        present: &["abp-appservice::BookAppService.GetCountAsync"],
        absent: &[],
    },
    Case {
        name: "abp/csharp",
        files: &[(
            "Mod.cs",
            "using Volo.Abp.Modularity;\npublic class BookModule : AbpModule {\n  public override void ConfigureServices(ServiceConfigurationContext c) {}\n  private void Helper() {}\n}\n",
        )],
        coverage: "abp (lifecycle, 1 entry)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "akka-net/csharp",
        files: &[(
            "Greeter.cs",
            "using Akka.Actor;\npublic class Greeter : ReceiveActor {\n  public Greeter() { Receive<int>(OnInt); }\n  private void OnInt(int i) {}\n  protected override void PreStart() {}\n}\npublic class Echo : UntypedActor {\n  protected override void OnReceive(object m) {}\n}\n",
        )],
        coverage: "akka-net (queue, 3 entries)",
        present: &[
            "akka-net::Greeter.Greeter",
            "akka-net::Greeter.PreStart",
            "akka-net::Echo.OnReceive",
        ],
        absent: &["akka-net::Greeter.OnInt"],
    },
    Case {
        name: "akka-net/fsharp",
        files: &[(
            "Greeter.fs",
            "module Actors\nopen Akka.Actor\ntype Greeter() =\n    inherit ReceiveActor()\n    override x.PreStart() = ()\n",
        )],
        coverage: "akka-net (queue, 2 entries)",
        present: &["akka-net::Greeter.Greeter", "akka-net::Greeter.PreStart"],
        absent: &[],
    },
    Case {
        name: "grpc-dotnet/csharp-nested-base",
        files: &[(
            "BasketService.cs",
            "using Grpc.Core;\npublic class BasketService : Basket.BasketBase {\n  public override Task<R> GetBasket(Q q, ServerCallContext c) => null;\n}\npublic class Repo : Store.RepoBase {\n  public void Load() {}\n}\n",
        )],
        coverage: "grpc-dotnet (network, 1 entry)",
        present: &["grpc-dotnet::BasketService.GetBasket"],
        absent: &["grpc-dotnet::Repo.Load"],
    },
    Case {
        name: "grpc-dotnet/fsharp",
        files: &[(
            "Svc.fs",
            "module Svc\nopen Grpc.Core\ntype GreeterService() =\n    inherit Greeter.GreeterBase()\n    override x.SayHello(req, ctx) = Task.FromResult(HelloReply())\n",
        )],
        coverage: "grpc-dotnet (network,",
        present: &["grpc-dotnet::GreeterService.SayHello"],
        absent: &[],
    },
    Case {
        name: "orleans/fsharp",
        files: &[(
            "Grains.fs",
            "module Grains\nopen Orleans\ntype HelloGrain() =\n    inherit Grain()\n    interface IHello with\n        member _.SayHello(s) = Task.FromResult s\ntype Plain() =\n    member _.SayHello(s) = s\n",
        )],
        coverage: "orleans (network,",
        present: &["orleans::HelloGrain.SayHello"],
        absent: &["orleans::Plain.SayHello"],
    },
    Case {
        name: "aws-lambda/csharp-ilambdacontext",
        files: &[(
            "F.cs",
            "using Amazon.Lambda.Core;\npublic class Fn {\n  public string Handle(string input, ILambdaContext context) { Log(context, input, 1); return input; }\n  private void Log(ILambdaContext c, string m, int level) {}\n}\n",
        )],
        coverage: "aws-lambda (network, 1 entry)",
        present: &["aws-lambda::Fn.Handle"],
        absent: &["aws-lambda::Fn.Log"],
    },
    Case {
        name: "aws-lambda/fsharp-ilambdacontext",
        files: &[(
            "Lambda.fs",
            "module Lambda\nopen Amazon.Lambda.Core\nlet log (ctx: ILambdaContext) (m: string) (n: int) = ()\nlet PostFormat (request: string) (_context: ILambdaContext) =\n    log _context request 1\n    request\n",
        )],
        coverage: "aws-lambda (network, 1 entry)",
        present: &["aws-lambda::Lambda.PostFormat"],
        absent: &["aws-lambda::Lambda.log"],
    },
    // ---------------------------------------------------------------- Go
    Case {
        name: "terraform-plugin-framework/go",
        files: &[(
            "r.go",
            "package p\nimport \"github.com/hashicorp/terraform-plugin-framework/resource\"\nvar _ resource.Resource = &thing{}\ntype thing struct{}\nfunc (t *thing) Metadata() {}\nfunc (t *thing) Read() { t.read() }\nfunc (t *thing) read() {}\n",
        )],
        coverage: "terraform-plugin-framework (public, 2 entries)",
        present: &["thing.Metadata", "thing.Read"],
        absent: &["thing.read ⟨"],
    },
    Case {
        name: "controller-runtime/go",
        files: &[(
            "c.go",
            "package c\nimport ctrl \"sigs.k8s.io/controller-runtime\"\nvar _ = ctrl.NewManager\ntype R struct{}\nfunc (r *R) Reconcile() {}\nfunc (r *R) reconcile() {}\n",
        )],
        coverage: "controller-runtime (queue, 1 entry)",
        present: &["controller-runtime::c.R.Reconcile"],
        absent: &[],
    },
    Case {
        name: "prometheus-collector/go",
        files: &[(
            "m.go",
            "package m\nimport \"github.com/prometheus/client_golang/prometheus\"\ntype col struct{}\nfunc (c col) Describe(ch chan<- *prometheus.Desc) {}\nfunc (c col) Collect(ch chan<- prometheus.Metric) {}\n",
        )],
        coverage: "prometheus-collector (lifecycle, 2 entries)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "gorm/go",
        files: &[(
            "u.go",
            "package model\nimport \"gorm.io/gorm\"\ntype User struct{}\nfunc (u *User) BeforeCreate(tx *gorm.DB) error { return nil }\nfunc (User) TableName() string { return \"users\" }\nfunc (u *User) beforeCreate() {}\n",
        )],
        coverage: "gorm (lifecycle, 2 entries)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "ent-schema/go",
        files: &[(
            "s.go",
            "package schema\nimport \"entgo.io/ent\"\ntype User struct{ ent.Schema }\nfunc (User) Fields() []ent.Field { return nil }\nfunc (User) Edges() []ent.Edge { return nil }\nfunc (User) Hooks() []ent.Hook { return nil }\n",
        )],
        coverage: "ent-schema (lifecycle, 2 entries)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "testify-suite/go",
        files: &[(
            "x_test.go",
            "package x\nimport (\n\t\"testing\"\n\t\"github.com/stretchr/testify/suite\"\n)\ntype S struct{ suite.Suite }\nfunc (s *S) SetupTest() {}\nfunc (s *S) TearDownTest() {}\nfunc (s *S) TestThing() {}\nfunc TestS(t *testing.T) { suite.Run(t, new(S)) }\n",
        )],
        coverage: "testify-suite (test, 2 entries)",
        present: &[],
        absent: &[],
    },
    Case {
        name: "bubbletea/go",
        files: &[(
            "m.go",
            "package main\nimport tea \"github.com/charmbracelet/bubbletea\"\ntype model struct{}\nfunc (m model) Init() tea.Cmd { return nil }\nfunc (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return m, nil }\nfunc (m model) View() string { return \"\" }\nfunc init() {}\nfunc main() { tea.NewProgram(model{}).Run() }\n",
        )],
        coverage: "bubbletea (lifecycle, 3 entries)",
        present: &["bubbletea::main.model.Init", "bubbletea::main.model.View"],
        absent: &["bubbletea::main.init"],
    },
    Case {
        name: "watermill/go",
        files: &[(
            "w.go",
            "package w\nimport \"github.com/ThreeDotsLabs/watermill/message\"\nfunc handle(msg *message.Message) ([]*message.Message, error) { return nil, nil }\nfunc setup(router *message.Router) {\n\trouter.AddHandler(\"orders\", \"in\", nil, \"out\", nil, handle)\n}\n",
        )],
        coverage: "watermill (queue, 1 entry)",
        present: &["watermill::"],
        absent: &[],
    },
    Case {
        name: "nats/go",
        files: &[(
            "n.go",
            "package n\nimport \"github.com/nats-io/nats.go\"\nfunc onMsg(m *nats.Msg) {}\nfunc setup(nc *nats.Conn) {\n\tnc.Subscribe(\"orders.*\", onMsg)\n}\n",
        )],
        coverage: "nats (queue, 1 entry)",
        present: &["nats::"],
        absent: &[],
    },
    Case {
        name: "sarama/go",
        files: &[(
            "k.go",
            "package k\nimport \"github.com/IBM/sarama\"\ntype handler struct{}\nfunc (handler) Setup(sarama.ConsumerGroupSession) error { return nil }\nfunc (h handler) ConsumeClaim(s sarama.ConsumerGroupSession, c sarama.ConsumerGroupClaim) error { return nil }\n",
        )],
        coverage: "sarama (queue, 1 entry)",
        present: &["sarama::k.handler.ConsumeClaim"],
        absent: &[],
    },
    Case {
        name: "discordgo/go",
        files: &[(
            "d.go",
            "package main\nimport \"github.com/bwmarrin/discordgo\"\nfunc messageCreate(s *discordgo.Session, m *discordgo.MessageCreate) {}\nfunc main() {\n\tdg, _ := discordgo.New(\"Bot x\")\n\tdg.AddHandler(messageCreate)\n}\n",
        )],
        coverage: "discordgo (network, 1 entry)",
        present: &["main.messageCreate"],
        absent: &[],
    },
    Case {
        name: "httprouter/go",
        files: &[(
            "h.go",
            "package main\nimport (\n\t\"net/http\"\n\t\"github.com/julienschmidt/httprouter\"\n)\nfunc Index(w http.ResponseWriter, r *http.Request, _ httprouter.Params) {}\nfunc main() {\n\trouter := httprouter.New()\n\trouter.GET(\"/\", Index)\n}\n",
        )],
        coverage: "httprouter (network, 1 entry)",
        present: &["httprouter::"],
        absent: &[],
    },
    Case {
        name: "go-restful/go",
        files: &[(
            "r.go",
            "package r\nimport restful \"github.com/emicklei/go-restful/v3\"\nfunc getUser(req *restful.Request, resp *restful.Response) {}\nfunc register(ws *restful.WebService) {\n\tws.Route(ws.GET(\"/users/{id}\").To(getUser))\n}\n",
        )],
        coverage: "go-restful (network, 1 entry)",
        present: &["r.getUser"],
        absent: &[],
    },
];

#[test]
fn every_dotnet_and_go_row_enumerates() {
    let mut failures: Vec<String> = Vec::new();
    for case in CASES {
        let tmp = TempDir::new().unwrap();
        for (rel, body) in case.files {
            write(tmp.path(), rel, body);
        }
        let (g, err) = run(tmp.path());
        let mut problems: Vec<String> = Vec::new();
        if !err.contains(case.coverage) {
            problems.push(format!("coverage line lacks {:?}", case.coverage));
        }
        for p in case.present {
            if !g.contains(p) {
                problems.push(format!("graph lacks {p:?}"));
            }
        }
        for a in case.absent {
            if g.contains(a) {
                problems.push(format!("graph has {a:?}"));
            }
        }
        if !problems.is_empty() {
            failures.push(format!(
                "{}: {}\n--- stderr\n{err}\n--- graph\n{g}",
                case.name,
                problems.join("; ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n=====\n")
    );
}

/// Every Go rule compares method names with their case: the first
/// letter is Go's export bit, so a `serveHTTP` helper never satisfies
/// `http.Handler` and is not a `net-http` entry.
#[test]
fn go_method_rules_are_case_sensitive() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "a.go",
        "package main\nimport \"net/http\"\ntype h struct{}\nfunc (x h) serveHTTP(w http.ResponseWriter, r *http.Request) {}\nfunc (x h) ServeHTTP(w http.ResponseWriter, r *http.Request) {}\nfunc main() {}\n",
    );
    let (g, err) = run(tmp.path());
    assert!(err.contains("net-http (network, 1 entry)"), "{err}\n{g}");
    assert!(g.contains("net-http::main.h.ServeHTTP"), "{g}");
    assert!(!g.contains("net-http::main.h.serveHTTP"), "{g}");
}
