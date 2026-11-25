//! Framework rows for application frameworks of JavaScript/TypeScript,
//! Swift, Objective-C and Dart (the 0.14.0 framework audit).
//!
//! One case per rule and language: the smallest source carrying the
//! framework's real entry shape, the coverage line it must produce
//! (whose entry count also proves a decoy beside it was left alone), and
//! the entry-node text a node-minting rule must show.

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

/// Run cgg over `dir` and return (mermaid graph, stderr).
fn run(dir: &Path, extra: &[&str]) -> (String, String) {
    let out = dir.join("g.mmd");
    let mut cmd = Command::cargo_bin("cgg").expect("cgg binary");
    cmd.args(["-o"]).arg(&out).args(extra).arg(dir);
    let assert = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

struct Case {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// Substrings of stderr (the coverage table).
    coverage: &'static [&'static str],
    /// Substrings of the graph.
    graph: &'static [&'static str],
}

const CASES: &[Case] = &[
    // ---- JavaScript / TypeScript ----
    Case {
        name: "oclif/typescript",
        files: &[(
            "src/commands/hello.ts",
            "import { Command, Flags } from '@oclif/core';\n\
             export default class Hello extends Command {\n\
             \x20 async run(): Promise<void> { this.greet('x'); }\n\
             \x20 greet(n: string) { this.log(n); }\n\
             \x20 async catch(err: Error) { throw err; }\n\
             }\n",
        )],
        coverage: &["oclif (cli, 2 entries)"],
        graph: &["cli::oclif::Hello"],
    },
    Case {
        name: "oclif/javascript",
        files: &[(
            "src/commands/hello.js",
            "const { Command } = require('@oclif/core');\n\
             class Hello extends Command {\n\
             \x20 async run() { this.greet(); }\n\
             \x20 greet() {}\n\
             }\n\
             module.exports = Hello;\n",
        )],
        coverage: &["oclif (cli, 1 entry)"],
        graph: &["cli::oclif::"],
    },
    Case {
        name: "elysia/typescript",
        files: &[(
            "src/index.ts",
            "import { Elysia } from 'elysia';\n\
             function listUsers() { return []; }\n\
             function audit() {}\n\
             new Elysia().get('/users', listUsers).onRequest(audit).listen(3000);\n",
        )],
        coverage: &["elysia (network, 2 entries)"],
        graph: &["network::elysia::get('/users')"],
    },
    Case {
        name: "elysia/javascript",
        files: &[(
            "src/index.js",
            "import { Elysia } from 'elysia';\n\
             function listUsers() { return []; }\n\
             new Elysia().post('/users', listUsers).listen(3000);\n",
        )],
        coverage: &["elysia (network, 1 entry)"],
        graph: &["network::elysia::post('/users')"],
    },
    Case {
        name: "tsoa/typescript",
        files: &[(
            "src/users.ts",
            "import { Controller, Get, Post, Route } from 'tsoa';\n\
             @Route('users')\n\
             export class UsersController extends Controller {\n\
             \x20 @Get('{id}')\n\
             \x20 public async getUser(id: number) { return this.load(id); }\n\
             \x20 @Post()\n\
             \x20 public async create() { return 1; }\n\
             \x20 private load(id: number) { return id; }\n\
             }\n",
        )],
        coverage: &["tsoa (network, 2 entries)"],
        graph: &["network::tsoa::Get('{id}')"],
    },
    Case {
        name: "routing-controllers/typescript",
        files: &[(
            "src/users.ts",
            "import { JsonController, Get, Delete } from 'routing-controllers';\n\
             @JsonController('/users')\n\
             export class UserController {\n\
             \x20 @Get('/')\n\
             \x20 getAll() { return this.all(); }\n\
             \x20 @Delete('/:id')\n\
             \x20 remove() { return null; }\n\
             \x20 private all() { return []; }\n\
             }\n",
        )],
        coverage: &["routing-controllers (network, 2 entries)"],
        graph: &["network::routing-controllers::Delete('/:id')"],
    },
    Case {
        name: "inversify-express-utils/typescript",
        files: &[(
            "src/foo.ts",
            "import { controller, httpGet, httpPost } from 'inversify-express-utils';\n\
             @controller('/foo')\n\
             export class FooController {\n\
             \x20 @httpGet('/')\n\
             \x20 public index() { return this.svc(); }\n\
             \x20 @httpPost('/')\n\
             \x20 public create() { return 1; }\n\
             \x20 private svc() { return 0; }\n\
             }\n",
        )],
        coverage: &["inversify-express-utils (network, 2 entries)"],
        graph: &["network::inversify-express-utils::httpGet('/')"],
    },
    Case {
        name: "tsed/typescript",
        files: &[(
            "src/cal.ts",
            "import { Controller } from '@tsed/di';\n\
             import { Get, Put } from '@tsed/schema';\n\
             @Controller('/calendars')\n\
             export class CalendarCtrl {\n\
             \x20 @Get('/:id')\n\
             \x20 get() { return {}; }\n\
             \x20 @Put('/:id')\n\
             \x20 update() { return {}; }\n\
             }\n",
        )],
        coverage: &["tsed (network, 2 entries)"],
        graph: &["network::tsed::Get('/:id')"],
    },
    Case {
        name: "midway/typescript",
        files: &[(
            "src/home.ts",
            "import { Controller, Get, Del } from '@midwayjs/core';\n\
             @Controller('/')\n\
             export class HomeController {\n\
             \x20 @Get('/')\n\
             \x20 async home() { return 'ok'; }\n\
             \x20 @Del('/item')\n\
             \x20 async drop() { return 'ok'; }\n\
             }\n",
        )],
        coverage: &["midway (network, 2 entries)"],
        graph: &["network::midway::Del('/item')"],
    },
    Case {
        name: "type-graphql/typescript",
        files: &[(
            "src/recipe.ts",
            "import { Resolver, Query, Mutation, FieldResolver } from 'type-graphql';\n\
             @Resolver()\n\
             export class RecipeResolver {\n\
             \x20 @Query(() => [String])\n\
             \x20 recipes() { return this.fetch(); }\n\
             \x20 @Mutation(() => String)\n\
             \x20 addRecipe() { return ''; }\n\
             \x20 @FieldResolver()\n\
             \x20 rating() { return 0; }\n\
             \x20 private fetch() { return []; }\n\
             }\n",
        )],
        coverage: &["type-graphql (network, 3 entries)"],
        graph: &["network::type-graphql::"],
    },
    Case {
        name: "sequelize-typescript/typescript",
        files: &[(
            "server/models/User.ts",
            "import { Table, Model, BeforeCreate, AfterDestroy } from 'sequelize-typescript';\n\
             @Table\n\
             export class User extends Model {\n\
             \x20 @BeforeCreate\n\
             \x20 static hashPassword(u: User) { User.salt(); }\n\
             \x20 @AfterDestroy\n\
             \x20 static cleanup(u: User) {}\n\
             \x20 static salt() {}\n\
             }\n",
        )],
        coverage: &["sequelize-typescript (lifecycle, 2 entries)"],
        graph: &[],
    },
    Case {
        name: "typeorm-listeners+subscribers/typescript",
        files: &[(
            "src/user.ts",
            "import { Entity, BeforeInsert, AfterLoad, EventSubscriber, EntitySubscriberInterface, InsertEvent } from 'typeorm';\n\
             @Entity()\n\
             export class User {\n\
             \x20 @BeforeInsert()\n\
             \x20 setDates() {}\n\
             \x20 @AfterLoad()\n\
             \x20 compute() {}\n\
             }\n\
             @EventSubscriber()\n\
             export class UserSubscriber implements EntitySubscriberInterface<User> {\n\
             \x20 listenTo() { return User; }\n\
             \x20 beforeInsert(event: InsertEvent<User>) { this.audit(); }\n\
             \x20 audit() {}\n\
             }\n",
        )],
        coverage: &[
            "typeorm-listeners (lifecycle, 2 entries)",
            "typeorm-subscribers (lifecycle, 2 entries)",
        ],
        graph: &[],
    },
    Case {
        name: "phaser/javascript",
        files: &[(
            "src/main.js",
            "import Phaser from 'phaser';\n\
             export class Main extends Phaser.Scene {\n\
             \x20 preload() {}\n\
             \x20 create() { this.setup(); }\n\
             \x20 update(t, dt) {}\n\
             \x20 setup() {}\n\
             }\n\
             export class Factory { create() {} }\n",
        )],
        coverage: &["phaser (lifecycle, 3 entries)"],
        graph: &[],
    },
    Case {
        name: "phaser/typescript",
        files: &[(
            "src/main.ts",
            "import { Scene } from 'phaser';\n\
             export class Main extends Scene {\n\
             \x20 init(): void {}\n\
             \x20 create(): void {}\n\
             }\n",
        )],
        coverage: &["phaser (lifecycle, 2 entries)"],
        graph: &[],
    },
    Case {
        name: "obsidian/typescript",
        files: &[(
            "src/main.ts",
            "import { Plugin, Modal } from 'obsidian';\n\
             export default class MyPlugin extends Plugin {\n\
             \x20 async onload() { this.refresh(); }\n\
             \x20 onunload() {}\n\
             \x20 refresh() {}\n\
             }\n\
             class Ask extends Modal {\n\
             \x20 onOpen() {}\n\
             \x20 onClose() {}\n\
             }\n",
        )],
        coverage: &["obsidian (lifecycle, 4 entries)"],
        graph: &[],
    },
    Case {
        name: "obsidian/javascript",
        files: &[(
            "main.js",
            "const { Plugin } = require('obsidian');\n\
             module.exports = class MyPlugin extends Plugin {\n\
             \x20 onload() {}\n\
             };\n",
        )],
        coverage: &["obsidian (lifecycle, 1 entry)"],
        graph: &[],
    },
    Case {
        name: "lit/typescript",
        files: &[(
            "src/el.ts",
            "import { LitElement, html } from 'lit';\n\
             export class MyEl extends LitElement {\n\
             \x20 render() { return html`<p></p>`; }\n\
             \x20 firstUpdated() {}\n\
             \x20 connectedCallback() { super.connectedCallback(); }\n\
             }\n\
             export class Report { render() { return ''; } }\n",
        )],
        coverage: &["lit (lifecycle, 3 entries)"],
        graph: &[],
    },
    Case {
        name: "lit/javascript",
        files: &[(
            "src/el.js",
            "import { LitElement, html } from 'lit';\n\
             export class MyEl extends LitElement {\n\
             \x20 render() { return html`<p></p>`; }\n\
             }\n",
        )],
        coverage: &["lit (lifecycle, 1 entry)"],
        graph: &[],
    },
    // ---- Swift ----
    Case {
        name: "swift-argument-parser/swift",
        files: &[(
            "Sources/Tool/Tool.swift",
            "import ArgumentParser\n\
             @main\n\
             struct Tool: AsyncParsableCommand {\n\
             \x20 mutating func run() async throws { helper() }\n\
             \x20 func validate() throws {}\n\
             }\n\
             func helper() {}\n",
        )],
        coverage: &["swift-argument-parser (cli, 2 entries)"],
        graph: &["cli::swift-argument-parser::"],
    },
    Case {
        name: "fluent/swift",
        files: &[(
            "Sources/App/CreateUser.swift",
            "import Fluent\n\
             struct CreateUser: AsyncMigration {\n\
             \x20 func prepare(on database: Database) async throws {}\n\
             \x20 func revert(on database: Database) async throws {}\n\
             }\n",
        )],
        coverage: &["fluent (lifecycle, 2 entries)"],
        graph: &[],
    },
    Case {
        name: "vapor+vapor-commands/swift",
        files: &[(
            "Sources/App/Hello.swift",
            "import Vapor\n\
             struct Hello: AsyncCommand {\n\
             \x20 struct Signature: CommandSignature {}\n\
             \x20 func run(using context: CommandContext, signature: Signature) async throws {}\n\
             }\n\
             struct UserController: RouteCollection {\n\
             \x20 func boot(routes: RoutesBuilder) throws {}\n\
             }\n",
        )],
        coverage: &["vapor-commands (cli, 1 entry)", "vapor (network, 1 entry)"],
        graph: &["cli::vapor-commands::", "network::vapor::"],
    },
    Case {
        name: "vapor-queues/swift",
        files: &[(
            "Sources/App/Email.swift",
            "import Queues\n\
             struct Email: AsyncJob {\n\
             \x20 typealias Payload = String\n\
             \x20 func dequeue(_ context: QueueContext, _ payload: String) async throws {}\n\
             }\n\
             struct Cleanup: AsyncScheduledJob {\n\
             \x20 func run(context: QueueContext) async throws {}\n\
             }\n",
        )],
        coverage: &["vapor-queues (queue, 2 entries)"],
        graph: &["queue::vapor-queues::"],
    },
    Case {
        name: "widgetkit+appintents/swift",
        files: &[
            (
                "Widget/Provider.swift",
                "import WidgetKit\n\
                 struct Provider: TimelineProvider {\n\
                 \x20 func placeholder(in context: Context) -> Entry { Entry() }\n\
                 \x20 func getSnapshot(in context: Context, completion: @escaping (Entry) -> ()) {}\n\
                 \x20 func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> ()) {}\n\
                 }\n",
            ),
            (
                "Intents/Open.swift",
                "import AppIntents\n\
                 struct OpenIntent: AppIntent {\n\
                 \x20 func perform() async throws -> some IntentResult { .result() }\n\
                 }\n",
            ),
        ],
        coverage: &[
            "widgetkit (lifecycle, 3 entries)",
            "appintents (lifecycle, 1 entry)",
        ],
        graph: &[],
    },
    Case {
        name: "apple-app-extension/swift",
        files: &[(
            "NotificationService/NotificationService.swift",
            "import UserNotifications\n\
             class NotificationService: UNNotificationServiceExtension {\n\
             \x20 override func didReceive(_ request: UNNotificationRequest, withContentHandler h: @escaping (UNNotificationContent) -> Void) {}\n\
             \x20 override func serviceExtensionTimeWillExpire() {}\n\
             }\n",
        )],
        coverage: &["apple-app-extension (lifecycle, 2 entries)"],
        graph: &[],
    },
    Case {
        name: "appkit/swift",
        files: &[(
            "App/Canvas.swift",
            "import AppKit\n\
             class Canvas: NSView {\n\
             \x20 override func mouseDown(with event: NSEvent) { helper() }\n\
             \x20 override func draw(_ dirtyRect: NSRect) {}\n\
             \x20 func helper() {}\n\
             }\n\
             class Doc: NSDocument {\n\
             \x20 override func data(ofType typeName: String) throws -> Data { Data() }\n\
             }\n\
             class Plain { func draw() {} func data() {} }\n",
        )],
        coverage: &["appkit (lifecycle, 3 entries)"],
        graph: &[],
    },
    Case {
        name: "core-data/swift",
        files: &[(
            "App/Item.swift",
            "import CoreData\n\
             class Item: NSManagedObject {\n\
             \x20 override func awakeFromInsert() { super.awakeFromInsert() }\n\
             \x20 override func willSave() {}\n\
             }\n",
        )],
        coverage: &["core-data (lifecycle, 2 entries)"],
        graph: &[],
    },
    // ---- Objective-C ----
    Case {
        name: "react-native/objc",
        files: &[
            (
                "ios/CalendarModule.h",
                "#import <React/RCTBridgeModule.h>\n\
                 @interface CalendarModule : NSObject <RCTBridgeModule>\n@end\n",
            ),
            (
                "ios/CalendarModule.m",
                "#import \"CalendarModule.h\"\n\
                 #import <React/RCTLog.h>\n\
                 @implementation CalendarModule\n\
                 RCT_EXPORT_MODULE();\n\
                 + (BOOL)requiresMainQueueSetup\n{\n  return NO;\n}\n\
                 - (NSArray<NSString *> *)supportedEvents\n{\n  return @[@\"onChange\"];\n}\n\
                 - (void)helper {}\n\
                 RCT_EXPORT_METHOD(createCalendarEvent:(NSString *)name location:(NSString *)location)\n\
                 {\n  RCTLogInfo(@\"Pretending to create an event %@ at %@\", name, location);\n  [self helper];\n}\n\
                 RCT_REMAP_METHOD(findEvents,\n                 findEventsWithResolver:(RCTPromiseResolveBlock)resolve\n                 rejecter:(RCTPromiseRejectBlock)reject)\n\
                 {\n  resolve(@[]);\n}\n\
                 RCT_EXPORT_BLOCKING_SYNCHRONOUS_METHOD(getName)\n{\n  return @\"x\";\n}\n\
                 @end\n",
            ),
        ],
        coverage: &["react-native (ffi, 5 entries)"],
        graph: &[
            "ffi::react-native::CalendarModule::createCalendarEvent",
            "ffi::react-native::CalendarModule::findEventsWithResolver",
        ],
    },
    Case {
        name: "appkit/objc",
        files: &[(
            "Doc.m",
            "#import <Cocoa/Cocoa.h>\n\
             @interface Doc : NSDocument\n@end\n\
             @implementation Doc\n\
             + (BOOL)autosavesInPlace { return YES; }\n\
             - (NSString *)windowNibName { return @\"Doc\"; }\n\
             - (NSData *)dataOfType:(NSString *)t error:(NSError **)e { return nil; }\n\
             - (void)helper {}\n\
             @end\n\
             @interface V : NSView\n@end\n\
             @implementation V\n\
             - (void)mouseDown:(NSEvent *)e {}\n\
             - (BOOL)isFlipped { return YES; }\n\
             - (BOOL)validateMenuItem:(NSMenuItem *)m { return YES; }\n\
             @end\n",
        )],
        coverage: &["appkit (lifecycle, 6 entries)"],
        graph: &[],
    },
    Case {
        name: "core-data/objc",
        files: &[(
            "Item.m",
            "#import <CoreData/CoreData.h>\n\
             @interface Item : NSManagedObject\n@end\n\
             @implementation Item\n\
             - (void)awakeFromInsert { [super awakeFromInsert]; }\n\
             - (void)willSave { [super willSave]; }\n\
             - (BOOL)validateForInsert:(NSError **)error { return YES; }\n\
             - (void)helper {}\n\
             @end\n\
             @interface ListVC : NSObject <NSFetchedResultsControllerDelegate>\n@end\n\
             @implementation ListVC\n\
             - (void)controllerDidChangeContent:(NSFetchedResultsController *)c {}\n\
             @end\n",
        )],
        coverage: &["core-data (lifecycle, 4 entries)"],
        graph: &[],
    },
    // ---- Dart ----
    Case {
        name: "equatable/dart",
        files: &[(
            "lib/point.dart",
            "import 'package:equatable/equatable.dart';\n\
             class Point extends Equatable {\n\
             \x20 final int x;\n\
             \x20 const Point(this.x);\n\
             \x20 @override\n\
             \x20 List<Object?> get props => [x];\n\
             }\n\
             class Pair with EquatableMixin {\n\
             \x20 @override\n\
             \x20 List<Object?> get props => [];\n\
             \x20 @override\n\
             \x20 bool? get stringify => true;\n\
             }\n\
             class NotEq {\n\
             \x20 List<Object?> get props => [];\n\
             }\n",
        )],
        coverage: &["equatable (lifecycle, 3 entries)"],
        graph: &[],
    },
    Case {
        name: "getx/dart",
        files: &[(
            "lib/home.dart",
            "import 'package:get/get.dart';\n\
             class HomeController extends GetxController {\n\
             \x20 @override\n\
             \x20 void onInit() { super.onInit(); load(); }\n\
             \x20 @override\n\
             \x20 void onClose() {}\n\
             \x20 void load() {}\n\
             }\n\
             mixin LiveMixin on GetxController {\n\
             \x20 @override\n\
             \x20 void onReady() {}\n\
             }\n\
             class AudioController extends GetxController\n\
             \x20   with LiveMixin {\n\
             \x20 @override\n\
             \x20 void onInit() {}\n\
             }\n\
             class HomeBinding extends Bindings {\n\
             \x20 @override\n\
             \x20 void dependencies() {}\n\
             }\n",
        )],
        coverage: &["getx (lifecycle, 5 entries)"],
        graph: &[],
    },
    Case {
        name: "riverpod/dart",
        files: &[(
            "lib/counter.dart",
            "import 'package:flutter_riverpod/flutter_riverpod.dart';\n\
             class Counter extends Notifier<int> {\n\
             \x20 @override\n\
             \x20 int build() => 0;\n\
             }\n\
             class Logger extends ProviderObserver {\n\
             \x20 @override\n\
             \x20 void didAddProvider(ProviderBase p, Object? v, ProviderContainer c) {}\n\
             }\n\
             class ReportBuilder {\n\
             \x20 String build() => '';\n\
             }\n",
        )],
        coverage: &["riverpod (lifecycle, 2 entries)"],
        graph: &[],
    },
    Case {
        name: "dart-frog/dart",
        files: &[
            (
                "routes/index.dart",
                "import 'package:dart_frog/dart_frog.dart';\n\
                 Response onRequest(RequestContext context) => Response(body: 'hi');\n",
            ),
            (
                "routes/users/[id].dart",
                "import 'package:dart_frog/dart_frog.dart';\n\
                 Response onRequest(RequestContext context, String id) => Response(body: id);\n",
            ),
            (
                "routes/_middleware.dart",
                "import 'package:dart_frog/dart_frog.dart';\n\
                 Handler middleware(Handler handler) => handler;\n",
            ),
            (
                "lib/helpers.dart",
                "import 'package:dart_frog/dart_frog.dart';\n\
                 Response onRequest(RequestContext context) => Response(body: 'not a route');\n",
            ),
        ],
        coverage: &["dart-frog (network, 3 entries)"],
        graph: &[
            "network::dart-frog::/users/[id]",
            "network::dart-frog::/_middleware",
        ],
    },
    Case {
        name: "dart-args/dart",
        files: &[(
            "lib/commands.dart",
            "import 'package:args/command_runner.dart';\n\
             abstract class BaseCmd extends Command<int> {\n\
             \x20 void log(String m) {}\n\
             }\n\
             class ListCmd extends BaseCmd {\n\
             \x20 @override\n\
             \x20 String get name => 'list';\n\
             \x20 @override\n\
             \x20 Future<int> run() async { log('x'); return 0; }\n\
             }\n",
        )],
        coverage: &["dart-args (cli, 1 entry)"],
        graph: &["cli::dart-args::ListCmd"],
    },
    // ---- AI agent and tool-calling frameworks (kind `tool`) ----
    Case {
        name: "mcp-sdk+mcp-handler/typescript",
        files: &[
            (
                "src/server.ts",
                "import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';\n\
                 import { z } from 'zod';\n\
                 const server = new McpServer({ name: 'fx', version: '1.0.0' });\n\
                 server.tool('echo', 'Echo text', { text: z.string() }, async ({ text }) => ({ content: [{ type: 'text', text: shout(text) }] }));\n\
                 server.registerTool('weather', { description: 'w' }, getWeather);\n\
                 server.registerResource('user', 'users://{id}', { title: 'u' }, readUser);\n\
                 server.prompt('review', { code: z.string() }, reviewPrompt);\n\
                 function shout(s: string) { return s.toUpperCase(); }\n\
                 async function getWeather(a: unknown) { return { content: [] }; }\n\
                 async function readUser(uri: URL) { return { contents: [] }; }\n\
                 function reviewPrompt(a: unknown) { return { messages: [] }; }\n",
            ),
            (
                "app/api/route.ts",
                "import { createMcpHandler } from 'mcp-handler';\n\
                 const handler = createMcpHandler((server) => { setup(server); }, {}, { basePath: '/api' });\n\
                 function setup(s: unknown) {}\n\
                 export { handler as GET };\n",
            ),
        ],
        coverage: &[
            "mcp-sdk (tool, 4 entries)",
            "mcp-handler (lifecycle, 1 entry)",
        ],
        graph: &[
            "tool::mcp-sdk::echo",
            "tool::mcp-sdk::weather",
            "tool::mcp-sdk::user",
            "tool::mcp-sdk::review",
        ],
    },
    Case {
        name: "mcp-sdk/javascript",
        files: &[(
            "server.js",
            "import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';\n\
             const server = new McpServer({ name: 'fx', version: '1.0.0' });\n\
             server.registerTool('lookup', { description: 'l' }, lookup);\n\
             async function lookup(a) { return { content: [] }; }\n",
        )],
        coverage: &["mcp-sdk (tool, 1 entry)"],
        graph: &["tool::mcp-sdk::lookup"],
    },
    Case {
        name: "cloudflare-agents+workers-rpc/typescript",
        files: &[(
            "src/server.ts",
            "import { Agent, callable } from 'agents';\n\
             import { WorkerEntrypoint } from 'cloudflare:workers';\n\
             export class MyAgent extends Agent<Env, { count: number }> {\n\
             \x20 async onStart() { await this.schedule(60, 'cleanup'); }\n\
             \x20 async onRequest(request: Request) { return new Response(''); }\n\
             \x20 @callable()\n\
             \x20 async increment(by: number) { return by; }\n\
             \x20 async cleanup() {}\n\
             \x20 private helperNeverCalled() {}\n\
             }\n\
             export class Rpc extends WorkerEntrypoint {\n\
             \x20 async add(a: number, b: number) { return a + b; }\n\
             \x20 private secret() {}\n\
             }\n",
        )],
        coverage: &[
            "cloudflare-agents (tool, 1 entry)",
            "cloudflare-agents-lifecycle (lifecycle, 2 entries)",
            "cloudflare-workers-rpc (network, 1 entry)",
        ],
        graph: &[
            "tool::cloudflare-agents::increment",
            "network::cloudflare-workers-rpc::Rpc.add",
        ],
    },
    Case {
        name: "vercel-ai/typescript",
        files: &[(
            "agent.ts",
            "import { tool, generateText } from 'ai';\n\
             import { z } from 'zod';\n\
             async function lookupStock({ symbol }: { symbol: string }) { return symbol; }\n\
             export const stockTool = tool({ description: 'stock', inputSchema: z.object({ symbol: z.string() }), execute: lookupStock });\n\
             export async function main() {\n\
             \x20 return generateText({ model: 'm', tools: { stock: stockTool }, onStepFinish: logStep, prompt: 'hi' });\n\
             }\n\
             function logStep(step: unknown) {}\n",
        )],
        coverage: &[
            "vercel-ai (tool, 1 entry)",
            "vercel-ai-hooks (lifecycle, 1 entry)",
        ],
        graph: &["tool::vercel-ai::lookupStock"],
    },
    Case {
        name: "mastra/typescript",
        files: &[(
            "index.ts",
            "import { Agent } from '@mastra/core/agent';\n\
             import { createTool } from '@mastra/core/tools';\n\
             import { createStep } from '@mastra/core/workflows';\n\
             import { z } from 'zod';\n\
             export const weatherTool = createTool({ id: 'get-weather', description: 'w', inputSchema: z.object({}), async execute({ context }) { return helper(context); } });\n\
             function helper(c: unknown) { return c; }\n\
             async function plan({ inputData }: any) { return inputData; }\n\
             const planStep = createStep({ id: 'plan', inputSchema: z.object({}), outputSchema: z.object({}), execute: plan });\n\
             export const agent = new Agent({ name: 'A', instructions: dynamicInstructions, model: 'm' });\n\
             function dynamicInstructions(c: unknown) { return 'x'; }\n",
        )],
        coverage: &[
            "mastra (tool, 1 entry)",
            "mastra-workflows (lifecycle, 1 entry)",
            "mastra-agent (lifecycle, 1 entry)",
        ],
        graph: &["tool::mastra::get-weather", "lifecycle::mastra-workflows::"],
    },
    Case {
        name: "langchain-js/typescript",
        files: &[(
            "tools.ts",
            "import { tool, StructuredTool } from '@langchain/core/tools';\n\
             import { BaseCallbackHandler } from '@langchain/core/callbacks/base';\n\
             import { z } from 'zod';\n\
             async function searchDocs({ q }: { q: string }) { return q; }\n\
             export const search = tool(searchDocs, { name: 'search', schema: z.object({ q: z.string() }) });\n\
             export class WikiTool extends StructuredTool {\n\
             \x20 name = 'wiki';\n\
             \x20 async _call(input: { q: string }) { return this.fetchPage(input.q); }\n\
             \x20 private fetchPage(q: string) { return q; }\n\
             }\n\
             export class MyHandler extends BaseCallbackHandler {\n\
             \x20 name = 'h';\n\
             \x20 async handleLLMStart() {}\n\
             \x20 async handleToolEnd(output: string) {}\n\
             }\n",
        )],
        coverage: &[
            "langchain-js (tool, 2 entries)",
            "langchain-js-callbacks (lifecycle, 2 entries)",
        ],
        graph: &["tool::langchain-js::search", "tool::langchain-js::WikiTool"],
    },
    Case {
        name: "openai-agents/typescript",
        files: &[(
            "agents.ts",
            "import { Agent, tool, handoff, defineInputGuardrail } from '@openai/agents';\n\
             import { z } from 'zod';\n\
             async function refundExecute(input: { id: string }) { return input.id; }\n\
             const refund = tool({ name: 'refund', description: 'r', parameters: z.object({ id: z.string() }), execute: refundExecute });\n\
             const guard = defineInputGuardrail({ name: 'in', execute: checkIn });\n\
             function checkIn(i: unknown) { return { outputInfo: {}, tripwireTriggered: false }; }\n\
             const billing = new Agent({ name: 'Billing', instructions: billingPrompt, tools: [refund] });\n\
             function billingPrompt(ctx: any) { return 'billing'; }\n\
             export const triage = Agent.create({ name: 'T', handoffs: [handoff(billing, { onHandoff: onBillingHandoff })], inputGuardrails: [guard] });\n\
             function onBillingHandoff(ctx: any) {}\n",
        )],
        coverage: &[
            "openai-agents (tool, 1 entry)",
            "openai-agents-handoffs (lifecycle, 1 entry)",
            "openai-agents-hooks (lifecycle, 2 entries)",
        ],
        graph: &[
            "tool::openai-agents::refund",
            "lifecycle::openai-agents-handoffs::",
        ],
    },
    Case {
        name: "claude-agent-sdk/typescript",
        files: &[(
            "server.ts",
            "import { tool, createSdkMcpServer } from '@anthropic-ai/claude-agent-sdk';\n\
             import { z } from 'zod';\n\
             const calc = tool('calculate', 'Do math', { expr: z.string() }, async (args) => ({ content: [{ type: 'text', text: evaluate(args.expr) }] }));\n\
             function evaluate(e: string) { return e; }\n\
             async function lookupHandler(args: { id: string }) { return { content: [] }; }\n\
             const lookup = tool('lookup', 'Look up', { id: z.string() }, lookupHandler);\n\
             export const server = createSdkMcpServer({ name: 't', version: '1', tools: [calc, lookup] });\n",
        )],
        coverage: &["claude-agent-sdk (tool, 2 entries)"],
        graph: &[
            "tool::claude-agent-sdk::calculate",
            "tool::claude-agent-sdk::lookup",
        ],
    },
    Case {
        name: "vscode/typescript",
        files: &[(
            "src/extension.ts",
            "import * as vscode from 'vscode';\n\
             export function activate(context: vscode.ExtensionContext) {\n\
             \x20 context.subscriptions.push(vscode.lm.registerTool('fx_run', new RunTool()));\n\
             \x20 vscode.chat.createChatParticipant('fx.cat', handleChat);\n\
             }\n\
             export function deactivate() {}\n\
             class RunTool implements vscode.LanguageModelTool<{ command: string }> {\n\
             \x20 async prepareInvocation(options: unknown) { return {}; }\n\
             \x20 async invoke(options: unknown, token: unknown) { return {}; }\n\
             }\n\
             async function handleChat(request: unknown) {}\n",
        )],
        coverage: &[
            "vscode-extension (lifecycle, 2 entries)",
            "vscode-lm-tools (tool, 3 entries)",
        ],
        graph: &[
            "tool::vscode-lm-tools::RunTool",
            "tool::vscode-lm-tools::fx.cat",
        ],
    },
    Case {
        name: "foundation-models/swift",
        files: &[(
            "App/Weather.swift",
            "import FoundationModels\n\
             struct GetWeatherTool: Tool {\n\
             \x20 let name = \"getWeather\"\n\
             \x20 func call(arguments: Arguments) async throws -> String { lookup(arguments.city) }\n\
             }\n\
             func lookup(_ c: String) -> String { c }\n\
             struct Plain { func call(arguments: Int) {} }\n",
        )],
        coverage: &["foundation-models (tool, 1 entry)"],
        graph: &["tool::foundation-models::GetWeatherTool"],
    },
];

#[test]
fn every_application_framework_row_enumerates() {
    let mut failures: Vec<String> = Vec::new();
    for case in CASES {
        let tmp = TempDir::new().unwrap();
        for (rel, body) in case.files {
            write(tmp.path(), rel, body);
        }
        let (g, err) = run(tmp.path(), &[]);
        for want in case.coverage {
            if !err.contains(want) {
                failures.push(format!("{}: coverage lacks {want:?}\n{err}", case.name));
            }
        }
        for want in case.graph {
            if !g.contains(want) {
                failures.push(format!("{}: graph lacks {want:?}\n{g}", case.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failure(s):\n\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// A JS/TS class extends the name its file imported, which a default
/// import or an `as` rename makes different from the name the base class
/// was declared under; the base chain must walk through the rename.
#[test]
fn a_renamed_or_default_imported_base_class_reaches_the_framework_base() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/lib/base.ts",
        "import { Command } from '@oclif/core';\n\
         export default abstract class BaseDataCommand extends Command {\n\
         \x20 protected header() {}\n\
         }\n",
    );
    write(
        tmp.path(),
        "src/lib/base2.ts",
        "import { Command } from '@oclif/core';\n\
         export abstract class OtherBase extends Command {\n\
         \x20 protected footer() {}\n\
         }\n",
    );
    write(
        tmp.path(),
        "src/commands/fork.ts",
        "import BaseCommand from '../lib/base.js';\n\
         export default class Fork extends BaseCommand {\n\
         \x20 async run() { helper(); }\n\
         }\n\
         function helper() {}\n",
    );
    write(
        tmp.path(),
        "src/commands/info.ts",
        "import { OtherBase as BC } from '../lib/base2';\n\
         export default class Info extends BC {\n\
         \x20 async run() {}\n\
         }\n",
    );
    let (g, err) = run(tmp.path(), &[]);
    assert!(err.contains("oclif (cli, 2 entries)"), "{err}");
    assert!(g.contains("cli::oclif::Fork"), "{g}");
    assert!(g.contains("cli::oclif::Info"), "{g}");
}

/// A JS/TS function imported from a project module and called or named
/// at module top level is credited: `makeApp({ register: registerTools })`
/// and `register('x', h.searchDocs)` used to leave every name dead.
#[test]
fn a_module_scope_use_of_an_imported_function_credits_it() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib.ts",
        "export function makeApp(o: { register: (c: unknown) => void }) { return o; }\n\
         export function registerTools(c: unknown) {}\n\
         export function unusedHelper() {}\n",
    );
    write(
        tmp.path(),
        "app.ts",
        "import { makeApp as mk, registerTools } from './lib';\n\
         export const app = mk({ register: registerTools });\n",
    );
    write(
        tmp.path(),
        "handlers.js",
        "export function searchDocs(q) { return q; }\n",
    );
    write(
        tmp.path(),
        "server.js",
        "import * as h from './handlers.js';\n\
         import { register } from 'somewhere';\n\
         register('search', h.searchDocs);\n",
    );
    let mut cmd = Command::cargo_bin("cgg").expect("cgg binary");
    cmd.args([
        "--dead-code",
        "--dead-code-format",
        "json",
        "--dead-code-confidence",
        "low",
        "--no-graph",
    ])
    .arg(tmp.path());
    let out = cmd.assert().success().get_output().stdout.clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let dead: Vec<&str> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    for live in ["makeApp", "registerTools", "searchDocs"] {
        assert!(!dead.contains(&live), "{live} reported dead: {dead:?}");
    }
    assert!(dead.contains(&"unusedHelper"), "{dead:?}");
}
