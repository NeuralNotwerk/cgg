//! The built-in framework rule table.
//!
//! Ranked by the inventory in §4 of the design: popularity from the
//! Stack Overflow 2025 Developer Survey plus registry download counts,
//! not by how easy each was to implement.
//!
//! Two things make this table honest rather than merely large.
//!
//! **Every rule is gated on detection.** `detect` lists import prefixes
//! that prove the framework is actually in use. Without that gate, a
//! rule matching the verb `get` would claim every `cache.get` in the
//! repository as an HTTP route. Detection is what lets the verb lists
//! stay short and shared — one `receiver.VERB(string, callable)` matcher
//! covers Gin, Echo, Fiber and Chi at once precisely because each is
//! gated on its own import.
//!
//! **Frameworks cgg cannot enumerate are still listed.** A rule with no
//! matchers contributes no entry nodes but does contribute a line to the
//! coverage table saying so. That is the difference between "cgg found
//! 3 routes" and "cgg found 3 routes and cannot read `config/routes.rb`,
//! where the other 300 live".

use super::{FrameworkRule, TrustKind};

/// Compact literal form of a rule. Expanded into owned [`FrameworkRule`]
/// values once, on first use.
#[derive(Debug)]
pub struct RuleSpec {
    pub id: &'static str,
    pub language: &'static str,
    pub kind: TrustKind,
    pub detect: &'static [&'static str],
    pub detect_paths: &'static [&'static str],
    /// Call names whose mere presence proves the framework — the escape
    /// hatch for ecosystems with no import records at all (WordPress).
    pub detect_calls: &'static [&'static str],
    pub attributes: &'static [&'static str],
    pub registrars: &'static [&'static str],
    pub base_types: &'static [&'static str],
    pub methods: &'static [&'static str],
    /// Whether a string argument may name the handler — shape E.
    pub string_targets: bool,
    pub node: bool,
    /// Stated when the rule cannot enumerate entries, for the coverage
    /// table's "seen, no rules" column.
    pub gap: &'static str,
}

const NONE: &[&str] = &[];

/// HTTP verbs, lowercase. Matching is case-insensitive so this one list
/// covers Go's `GET`, Rails' `get` and NestJS's `Get`.
const HTTP_VERBS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "trace", "connect",
];

pub const SPECS: &[RuleSpec] = &[
    // ---------------------------------------------------------------
    // Python
    // ---------------------------------------------------------------
    RuleSpec {
        id: "fastapi",
        language: "python",
        kind: TrustKind::Network,
        detect: &["fastapi"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "get",
            "post",
            "put",
            "delete",
            "patch",
            "head",
            "options",
            "route",
            "api_route",
            "websocket",
            "middleware",
            "exception_handler",
        ],
        registrars: &["add_api_route", "add_route", "add_websocket_route"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "flask",
        language: "python",
        kind: TrustKind::Network,
        detect: &["flask"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "route",
            "get",
            "post",
            "put",
            "delete",
            "patch",
            "errorhandler",
            "before_request",
            "after_request",
            "teardown_request",
            "websocket",
        ],
        registrars: &["add_url_rule"],
        base_types: &["MethodView", "Resource"],
        methods: HTTP_VERBS,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "django",
        language: "python",
        kind: TrustKind::Network,
        // `urls.py` is a plain Python module, so the routes are ordinary
        // calls — `path("users/", views.list_users)`. No file format to
        // parse, only a registrar to recognise.
        detect: &["django"],
        detect_paths: &["urls.py"],
        detect_calls: NONE,
        attributes: &[
            "require_http_methods",
            "api_view",
            "login_required",
            "csrf_exempt",
        ],
        registrars: &["path", "re_path", "url", "register"],
        base_types: &[
            "View",
            "TemplateView",
            "ListView",
            "DetailView",
            "CreateView",
            "UpdateView",
            "DeleteView",
            "APIView",
            "ViewSet",
            "ModelViewSet",
            "GenericAPIView",
            "ReadOnlyModelViewSet",
        ],
        methods: &[
            "get",
            "post",
            "put",
            "patch",
            "delete",
            "head",
            "options",
            "list",
            "create",
            "retrieve",
            "update",
            "partial_update",
            "destroy",
        ],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "django-admin",
        language: "python",
        kind: TrustKind::Cli,
        detect: &["django.core.management"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &["BaseCommand", "AppCommand", "LabelCommand"],
        methods: &["handle", "add_arguments"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "celery",
        language: "python",
        kind: TrustKind::Queue,
        detect: &["celery"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["task", "shared_task", "periodic_task", "on_after_configure"],
        registrars: NONE,
        base_types: &["Task"],
        methods: &["run"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "click",
        language: "python",
        kind: TrustKind::Cli,
        detect: &["click", "typer"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["command", "group", "callback"],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "torch",
        language: "python",
        kind: TrustKind::Lifecycle,
        detect: &["torch", "pytorch_lightning", "lightning"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &[
            "nn.Module",
            "Module",
            "LightningModule",
            "Dataset",
            "IterableDataset",
        ],
        methods: &[
            "forward",
            "training_step",
            "validation_step",
            "test_step",
            "predict_step",
            "configure_optimizers",
            "__getitem__",
            "__len__",
        ],
        // §8: one `torch:Module.__call__` node fanning out to every
        // model in the repo is visually useless. Root-mark instead.
        string_targets: false,
        node: false,
        gap: "",
    },
    // ---------------------------------------------------------------
    // JavaScript / TypeScript
    // ---------------------------------------------------------------
    RuleSpec {
        id: "express",
        language: "javascript",
        kind: TrustKind::Network,
        detect: &["express"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "get", "post", "put", "delete", "patch", "head", "options", "all", "use",
            "route",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "express",
        language: "typescript",
        kind: TrustKind::Network,
        detect: &["express"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "get", "post", "put", "delete", "patch", "head", "options", "all", "use",
            "route",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "nestjs",
        language: "typescript",
        kind: TrustKind::Network,
        detect: &["@nestjs/common", "@nestjs/core", "@nestjs/microservices"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "Get",
            "Post",
            "Put",
            "Delete",
            "Patch",
            "Head",
            "Options",
            "All",
            "MessagePattern",
            "EventPattern",
            "Sse",
            "SubscribeMessage",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "nestjs-schedule",
        language: "typescript",
        kind: TrustKind::Schedule,
        detect: &["@nestjs/schedule"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["Cron", "Interval", "Timeout"],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "bullmq",
        language: "javascript",
        kind: TrustKind::Queue,
        detect: &["bullmq", "bull"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["Worker", "process", "on"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "bullmq",
        language: "typescript",
        kind: TrustKind::Queue,
        detect: &["bullmq", "bull"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["Worker", "process", "on"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "worker-threads",
        language: "javascript",
        kind: TrustKind::Queue,
        detect: &["worker_threads", "piscina", "node:worker_threads"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        // `new Worker('./w.js')` names a whole module, not a callable.
        // The engine resolves the path to that module's top-level
        // callables — otherwise the entire worker file reads as dead.
        registrars: &["Worker", "Piscina"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "worker-threads",
        language: "typescript",
        kind: TrustKind::Queue,
        detect: &["worker_threads", "piscina", "node:worker_threads"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["Worker", "Piscina"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "nextjs",
        language: "javascript",
        kind: TrustKind::Network,
        detect: &["next", "next/"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "routes come from file-system layout, not from anything written in source",
    },
    RuleSpec {
        id: "nextjs",
        language: "typescript",
        kind: TrustKind::Network,
        detect: &["next", "next/"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "routes come from file-system layout, not from anything written in source",
    },
    // ---------------------------------------------------------------
    // Java / Kotlin
    // ---------------------------------------------------------------
    RuleSpec {
        id: "spring",
        language: "java",
        kind: TrustKind::Network,
        detect: &["org.springframework.web", "org.springframework.boot"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "RequestMapping",
            "GetMapping",
            "PostMapping",
            "PutMapping",
            "DeleteMapping",
            "PatchMapping",
            "MessageMapping",
            "ExceptionHandler",
            "InitBinder",
            "ModelAttribute",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "spring-jobs",
        language: "java",
        kind: TrustKind::Schedule,
        detect: &["org.springframework.scheduling"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["Scheduled", "Async"],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "spring-messaging",
        language: "java",
        kind: TrustKind::Queue,
        detect: &[
            "org.springframework.kafka",
            "org.springframework.amqp",
            "org.springframework.jms",
        ],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "KafkaListener",
            "RabbitListener",
            "JmsListener",
            "StreamListener",
            // Spring AMQP's two-level idiom: `@RabbitListener(queues=…)`
            // on the class names the queue, and the *method* that
            // actually receives carries only `@RabbitHandler`. Matching
            // the listener alone marks the class and misses every
            // handler on it. `@KafkaHandler` is the same shape.
            "RabbitHandler",
            "KafkaHandler",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "jakarta-rs",
        language: "java",
        kind: TrustKind::Network,
        // One detector covers Quarkus, Jersey, RESTEasy, Helidon and
        // Jakarta EE — they all speak the same annotation vocabulary.
        detect: &["jakarta.ws.rs", "javax.ws.rs"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "Path",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "micronaut",
        language: "java",
        kind: TrustKind::Network,
        detect: &["io.micronaut.http.annotation", "io.micronaut.http"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "Get",
            "Post",
            "Put",
            "Delete",
            "Patch",
            "Head",
            "Options",
            "Controller",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "akka",
        language: "java",
        kind: TrustKind::Queue,
        detect: &["akka.actor"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &[
            "AbstractBehavior",
            "AbstractActor",
            "UntypedAbstractActor",
            "AbstractLoggingActor",
        ],
        // §3: lifecycle entries that are genuinely distinct — one per
        // actor — do earn a node, unlike `Module.forward`.
        methods: &["createReceive", "onReceive", "preStart", "postStop"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "java-concurrent",
        language: "java",
        kind: TrustKind::Lifecycle,
        detect: &["java.util.concurrent", "java.lang.Thread"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &["Runnable", "Callable", "Thread", "TimerTask"],
        methods: &["run", "call"],
        // Principled replacement for the crude LIFECYCLE name list: only
        // a type that actually declares the contract is marked.
        string_targets: false,
        node: false,
        gap: "",
    },
    RuleSpec {
        id: "spring",
        language: "kotlin",
        kind: TrustKind::Network,
        detect: &["org.springframework.web", "org.springframework.boot"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "RequestMapping",
            "GetMapping",
            "PostMapping",
            "PutMapping",
            "DeleteMapping",
            "PatchMapping",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    // ---------------------------------------------------------------
    // Go — one `receiver.VERB(string, callable)` matcher, four routers.
    // ---------------------------------------------------------------
    RuleSpec {
        id: "gin",
        language: "go",
        kind: TrustKind::Network,
        detect: &["github.com/gin-gonic/gin"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "Any", "Handle",
            "Use",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "echo",
        language: "go",
        kind: TrustKind::Network,
        detect: &["github.com/labstack/echo"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "Any", "Add",
            "Use",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "fiber",
        language: "go",
        kind: TrustKind::Network,
        detect: &["github.com/gofiber/fiber"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "Get", "Post", "Put", "Delete", "Patch", "Head", "Options", "All", "Add",
            "Use",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "chi",
        language: "go",
        kind: TrustKind::Network,
        detect: &["github.com/go-chi/chi"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "Get",
            "Post",
            "Put",
            "Delete",
            "Patch",
            "Head",
            "Options",
            "Handle",
            "HandleFunc",
            "Method",
            "Use",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "net-http",
        language: "go",
        kind: TrustKind::Network,
        detect: &["net/http"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["HandleFunc", "Handle"],
        // Go interfaces are structural: nothing in the source says
        // `Handler implements http.Handler`, so there is no base type to
        // match on and the method name is the only available signal.
        // Gated on `net/http` being imported, `ServeHTTP` is specific
        // enough to carry it — and this is exactly the finding the
        // design's measurement flagged as a false positive.
        base_types: NONE,
        methods: &["ServeHTTP"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "temporal",
        language: "go",
        kind: TrustKind::Queue,
        detect: &["go.temporal.io/sdk"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "RegisterWorkflow",
            "RegisterActivity",
            "ExecuteWorkflow",
            "ExecuteActivity",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    // ---------------------------------------------------------------
    // Ruby
    // ---------------------------------------------------------------
    RuleSpec {
        id: "rails",
        language: "ruby",
        kind: TrustKind::Network,
        detect: &["rails", "action_controller", "actionpack"],
        // Rails is discovered by file convention. `config/routes.rb` is
        // ordinary Ruby — `get 'photos', to: 'photos#index'` — so the
        // routes are reachable as calls; what is missing is the
        // string→symbol step, which the engine does.
        detect_paths: &["config/routes.rb", "app/controllers/"],
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "get",
            "post",
            "put",
            "patch",
            "delete",
            "root",
            "match",
            "resources",
            "resource",
            "namespace",
            "scope",
        ],
        base_types: &[
            "ApplicationController",
            "ActionController::Base",
            "ActionController::API",
        ],
        methods: &[
            "index", "show", "new", "edit", "create", "update", "destroy",
        ],
        string_targets: true,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "sinatra",
        language: "ruby",
        kind: TrustKind::Network,
        detect: &["sinatra"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["get", "post", "put", "patch", "delete", "options", "head"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "grape",
        language: "ruby",
        kind: TrustKind::Network,
        detect: &["grape"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["get", "post", "put", "patch", "delete", "route"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "sidekiq",
        language: "ruby",
        kind: TrustKind::Queue,
        // Rails autoloads, so a worker file typically carries no
        // `require 'sidekiq'` at all — the convention directory is the
        // only marker there is. Measured on Mastodon, whose ~90 workers
        // were invisible without this.
        detect: &["sidekiq"],
        detect_paths: &["app/workers/", "app/jobs/"],
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &[
            "Sidekiq::Job",
            "Sidekiq::Worker",
            "ApplicationJob",
            "ActiveJob::Base",
        ],
        methods: &["perform"],
        string_targets: false,
        node: true,
        gap: "",
    },
    // ---------------------------------------------------------------
    // PHP
    // ---------------------------------------------------------------
    RuleSpec {
        id: "laravel",
        language: "php",
        kind: TrustKind::Network,
        detect: &[
            "Illuminate\\Support\\Facades\\Route",
            "Illuminate\\Routing",
            "Illuminate",
        ],
        detect_paths: &["routes/web.php", "routes/api.php", "routes/console.php"],
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "get",
            "post",
            "put",
            "patch",
            "delete",
            "any",
            "match",
            "resource",
            "apiResource",
            "redirect",
            "view",
        ],
        base_types: &["Controller", "Command", "Job"],
        methods: &[
            "index", "show", "store", "update", "destroy", "create", "edit", "handle",
        ],
        string_targets: true,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "symfony",
        language: "php",
        kind: TrustKind::Network,
        detect: &[
            "Symfony\\Component\\Routing\\Attribute\\Route",
            "Symfony\\Component\\Routing\\Annotation\\Route",
            "Symfony\\Bundle",
            "Symfony\\Component\\HttpFoundation",
        ],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "Route",
            "Get",
            "Post",
            "Put",
            "Delete",
            "Patch",
            "AsCommand",
            "AsMessageHandler",
        ],
        registrars: NONE,
        base_types: &["AbstractController", "Command"],
        methods: &["execute", "__invoke"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "wordpress",
        language: "php",
        kind: TrustKind::Network,
        detect: NONE,
        detect_paths: &["wp-content/", "functions.php"],
        // PHP has no import records for WordPress at all — it is a
        // global-function ecosystem. The calls themselves are the only
        // available marker.
        detect_calls: &[
            "add_action",
            "add_filter",
            "register_rest_route",
            "add_shortcode",
        ],
        attributes: NONE,
        registrars: &[
            "add_action",
            "add_filter",
            "register_rest_route",
            "add_shortcode",
            "register_activation_hook",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: true,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "codeigniter",
        language: "php",
        kind: TrustKind::Network,
        detect: &["CodeIgniter"],
        detect_paths: &["app/Config/Routes.php"],
        detect_calls: NONE,
        attributes: NONE,
        registrars: &["get", "post", "put", "patch", "delete", "add", "match"],
        base_types: &["BaseController", "Controller"],
        methods: NONE,
        string_targets: true,
        node: true,
        gap: "",
    },
    // ---------------------------------------------------------------
    // C#
    // ---------------------------------------------------------------
    RuleSpec {
        id: "aspnet-mvc",
        language: "csharp",
        kind: TrustKind::Network,
        detect: &["Microsoft.AspNetCore.Mvc", "Microsoft.AspNetCore"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "HttpGet",
            "HttpPost",
            "HttpPut",
            "HttpDelete",
            "HttpPatch",
            "HttpHead",
            "HttpOptions",
            "Route",
        ],
        registrars: NONE,
        base_types: &["Controller", "ControllerBase", "PageModel"],
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "aspnet-minimal",
        language: "csharp",
        kind: TrustKind::Network,
        detect: &["Microsoft.AspNetCore.Builder", "Microsoft.AspNetCore"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: &[
            "MapGet",
            "MapPost",
            "MapPut",
            "MapDelete",
            "MapPatch",
            "MapMethods",
            "MapGroup",
            "MapHub",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "dotnet-hosting",
        language: "csharp",
        kind: TrustKind::Lifecycle,
        detect: &["Microsoft.Extensions.Hosting"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &["BackgroundService", "IHostedService"],
        methods: &["ExecuteAsync", "StartAsync", "StopAsync"],
        string_targets: false,
        node: false,
        gap: "",
    },
    RuleSpec {
        id: "quartz",
        language: "csharp",
        kind: TrustKind::Schedule,
        detect: &["Quartz"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &["IJob"],
        methods: &["Execute"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "masstransit",
        language: "csharp",
        kind: TrustKind::Queue,
        detect: &["MassTransit"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: &["IConsumer"],
        methods: &["Consume"],
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "blazor",
        language: "csharp",
        kind: TrustKind::Network,
        detect: &["Microsoft.AspNetCore.Components"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "`.razor` components carry their `@page` route in markup cgg does not parse",
    },
    // ---------------------------------------------------------------
    // Rust
    // ---------------------------------------------------------------
    RuleSpec {
        id: "axum",
        language: "rust",
        kind: TrustKind::Network,
        detect: &["axum", "utoipa_axum"],
        detect_paths: NONE,
        detect_calls: NONE,
        // `utoipa::path` is shape A and matters more than it looks: the
        // `utoipa-axum` pattern registers handlers through
        // `.routes(routes!(a, b, c))`, a proc-macro whose token tree
        // names them. cgg cannot see inside that, but every one of those
        // handlers carries its method and path in this attribute — on
        // crates.io, 48 macro invocations' worth.
        attributes: &["debug_handler", "utoipa::path"],
        registrars: &[
            "route", "get", "post", "put", "delete", "patch", "head", "options", "any",
            "nest",
        ],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "actix-web",
        language: "rust",
        kind: TrustKind::Network,
        detect: &["actix_web", "actix-web"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "get",
            "post",
            "put",
            "delete",
            "patch",
            "head",
            "options",
            "route",
            "main",
            "utoipa::path",
        ],
        registrars: &["route", "service", "to", "default_service"],
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "rocket",
        language: "rust",
        kind: TrustKind::Network,
        detect: &["rocket"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &[
            "get",
            "post",
            "put",
            "delete",
            "patch",
            "head",
            "options",
            "route",
            "catch",
            "utoipa::path",
        ],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "actix-actor",
        language: "rust",
        kind: TrustKind::Queue,
        detect: &["actix"],
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: NONE,
        base_types: &["Actor", "Handler", "StreamHandler"],
        registrars: NONE,
        methods: &["handle", "started", "stopped", "handle_stream"],
        string_targets: false,
        node: false,
        gap: "",
    },
    // ---------------------------------------------------------------
    // C / C++ — compute
    // ---------------------------------------------------------------
    RuleSpec {
        id: "cuda",
        language: "cpp",
        kind: TrustKind::Lifecycle,
        // §8: `tree-sitter-cpp` parses `saxpy<<<a,b>>>(args)` as nested
        // comparison operators, so the launch produces no edge at all.
        // Treating `__global__` as a root qualifier fixes the resulting
        // cascade without fighting the grammar.
        detect: NONE,
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["__global__"],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
    RuleSpec {
        id: "cuda",
        language: "c",
        kind: TrustKind::Lifecycle,
        detect: NONE,
        detect_paths: NONE,
        detect_calls: NONE,
        attributes: &["__global__"],
        registrars: NONE,
        base_types: NONE,
        methods: NONE,
        string_targets: false,
        node: true,
        gap: "",
    },
];

/// Expand the compact table into owned rules.
pub fn builtin() -> Vec<FrameworkRule> {
    SPECS
        .iter()
        .map(|s| FrameworkRule {
            id: s.id.to_string(),
            language: s.language.to_string(),
            kind: s.kind,
            detect: s.detect.iter().map(|x| x.to_string()).collect(),
            detect_paths: s.detect_paths.iter().map(|x| x.to_string()).collect(),
            attributes: s.attributes.iter().map(|x| x.to_string()).collect(),
            registrars: s.registrars.iter().map(|x| x.to_string()).collect(),
            base_types: s.base_types.iter().map(|x| x.to_string()).collect(),
            methods: s.methods.iter().map(|x| x.to_string()).collect(),
            string_targets: s.string_targets,
            node: s.node,
        })
        .collect()
}

/// Identifiers that mark a *file* as the thing the framework enters,
/// rather than marking a callable inside it.
///
/// Shape F normally reads the spawn site: `new Worker('./jobs/x.js')`
/// names the module, and the entry is minted on the far end. That only
/// works when the path is a literal. Real code writes
/// `new Worker(workerFile)` with a variable, and the worker file is
/// then referenced from nowhere — the whole module reads as dead.
///
/// A worker module identifies itself, though: it imports
/// `worker_threads` and talks to `parentPort`. That pair is the receive
/// side of the channel and appears in no spawner, so it is a precise
/// marker for "this file is entered as a thread".
///
/// Kept out of [`FrameworkRule`] for the same reason as
/// [`detect_calls_for`]: it selects a *file*, not a callable, so no
/// matcher field fits it and a user-authored rule has no business
/// declaring one.
pub fn self_module_markers_for(id: &str, language: &str) -> &'static [&'static str] {
    match (id, language) {
        ("worker-threads", "javascript") | ("worker-threads", "typescript") => {
            &["parentPort", "workerData"]
        }
        _ => NONE,
    }
}

/// Call names that alone prove a framework is present, for ecosystems
/// with no import records. Returned separately from [`FrameworkRule`]
/// because it is a detection input, not a matcher, and a user-authored
/// rule has no business declaring one.
pub fn detect_calls_for(id: &str, language: &str) -> &'static [&'static str] {
    SPECS
        .iter()
        .find(|s| s.id == id && s.language == language)
        .map(|s| s.detect_calls)
        .unwrap_or(NONE)
}

/// Why a built-in rule cannot enumerate entries, if it cannot.
pub fn gap_for(id: &str, language: &str) -> &'static str {
    SPECS
        .iter()
        .find(|s| s.id == id && s.language == language)
        .map(|s| s.gap)
        .unwrap_or("")
}

/// Languages in which cgg has at least one framework rule. Everything
/// else is disclosed in the coverage table's "no rules" line rather than
/// silently producing zero entries.
pub fn languages_with_rules() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = SPECS.iter().map(|s| s.language).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Every registrar verb any rule can match, lowercased and deduplicated.
///
/// Exists so the *extraction* layer can skip calls no rule could ever
/// use. Argument capture is otherwise paid on every call expression in
/// the tree, and measured on TypeORM that doubled the run: thousands of
/// `describe('...', () => {})` blocks are shaped exactly like a route
/// registration and are not one.
///
/// Gating here loses nothing a built-in rule could have matched — the
/// list is their union. User-authored rules add their own verbs through
/// `cgg_lang::set_extra_registrar_verbs`.
pub fn registrar_verbs() -> &'static [&'static str] {
    static VERBS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    VERBS.get_or_init(|| {
        let mut v: Vec<&'static str> = SPECS
            .iter()
            .flat_map(|s| s.registrars.iter().copied())
            .collect();
        v.sort_unstable_by_key(|a| a.to_ascii_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_rule_is_either_detectable_or_a_declared_gap() {
        for s in SPECS {
            let detectable = !s.detect.is_empty()
                || !s.detect_paths.is_empty()
                || !s.detect_calls.is_empty();
            // CUDA is the one rule keyed purely on a source marker: the
            // `__global__` qualifier IS the evidence, so it needs no
            // import gate.
            let marker_is_the_evidence = s.id == "cuda";
            assert!(
                detectable || marker_is_the_evidence,
                "{}/{} can never fire: no detect, no paths, no calls",
                s.language,
                s.id
            );
        }
    }

    #[test]
    fn a_rule_with_no_matchers_must_declare_why() {
        // Silence is the failure mode this whole feature exists to
        // avoid. A framework cgg lists but cannot enumerate has to say
        // so, or the coverage table reads as complete when it is not.
        for s in SPECS {
            let has_matchers = !s.attributes.is_empty()
                || !s.registrars.is_empty()
                || !s.base_types.is_empty()
                || !s.methods.is_empty();
            assert!(
                has_matchers || !s.gap.is_empty(),
                "{}/{} has no matchers and no stated gap",
                s.language,
                s.id
            );
        }
    }

    #[test]
    fn ids_are_unique_per_language() {
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        for s in SPECS {
            assert!(
                seen.insert((s.language, s.id)),
                "duplicate rule {}/{}",
                s.language,
                s.id
            );
        }
    }

    #[test]
    fn registrar_verbs_covers_every_rules_vocabulary() {
        let verbs = registrar_verbs();
        for s in SPECS {
            for r in s.registrars {
                assert!(
                    verbs.iter().any(|v| v.eq_ignore_ascii_case(r)),
                    "{}/{} registrar `{r}` is not in the extraction gate — \
                     the rule can never fire",
                    s.language,
                    s.id
                );
            }
        }
    }

    #[test]
    fn builtin_expands_every_spec() {
        assert_eq!(builtin().len(), SPECS.len());
        let flask = builtin()
            .into_iter()
            .find(|r| r.id == "flask")
            .expect("flask rule");
        assert_eq!(flask.kind, TrustKind::Network);
        assert!(flask.has_matchers());
    }

    #[test]
    fn the_inventorys_web_frameworks_are_all_present() {
        // §4's inventory is the contract. If a row is dropped from the
        // table, the coverage claim in the README stops being true.
        let ids: HashSet<&str> = SPECS.iter().map(|s| s.id).collect();
        for want in [
            "fastapi",
            "flask",
            "django",
            "celery",
            "torch",
            "express",
            "nestjs",
            "spring",
            "jakarta-rs",
            "micronaut",
            "akka",
            "gin",
            "echo",
            "fiber",
            "chi",
            "net-http",
            "temporal",
            "rails",
            "sinatra",
            "grape",
            "sidekiq",
            "laravel",
            "symfony",
            "wordpress",
            "codeigniter",
            "aspnet-mvc",
            "aspnet-minimal",
            "quartz",
            "masstransit",
            "axum",
            "actix-web",
            "rocket",
            "cuda",
        ] {
            assert!(
                ids.contains(want),
                "inventory framework `{want}` is missing"
            );
        }
    }

    #[test]
    fn bucket_d_lifecycle_rules_do_not_mint_nodes() {
        // §8: an entry node per bucket-D method is visually useless.
        for s in SPECS {
            if s.kind == TrustKind::Lifecycle && !s.base_types.is_empty() {
                assert!(
                    !s.node,
                    "{}/{} would fan out a lifecycle node",
                    s.language, s.id
                );
            }
        }
    }
}
