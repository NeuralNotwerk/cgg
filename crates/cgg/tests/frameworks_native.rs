//! Framework rules for the native languages added by the 0.14.0 framework
//! audit: Rust, C, C++, Zig, Fortran, VHDL, assembly, CMake and Nix.
//!
//! One fixture per rule row, asserting the entry count the coverage table
//! reports, plus the negative case each matcher change exists for (a
//! member `setup()` is not Arduino's, a bare `Service` base is not gRPC's,
//! a `QObject` outside a test file is not a QtTest class, a zbus signal
//! is not a D-Bus handler).

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

/// Run cgg with `--dead-code` over `files` and return stderr (the
/// framework coverage table and the dead-code report).
fn run(files: &[(&str, &str)]) -> String {
    let tmp = TempDir::new().unwrap();
    for (rel, body) in files {
        write(tmp.path(), rel, body);
    }
    let out = tmp.path().join("g.mmd");
    let assert = Command::cargo_bin("cgg")
        .expect("cgg binary")
        .args(["--dead-code", "--framework-coverage", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    String::from_utf8_lossy(&assert.get_output().stderr).to_string()
}

/// The coverage table names `rule` with exactly `n` entries.
fn assert_entries(err: &str, rule: &str, kind: &str, n: usize) {
    let unit = if n == 1 { "entry" } else { "entries" };
    let want = format!("{rule} ({kind}, {n} {unit})");
    assert!(err.contains(&want), "want `{want}` in:\n{err}");
}

// ---------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------

#[test]
fn gdext_roots_exported_members_and_engine_virtuals() {
    let err = run(&[(
        "src/lib.rs",
        "use godot::prelude::*;\n\
         #[derive(GodotClass)]\n#[class(base=Node2D)]\npub struct Player { speed: f64 }\n\
         #[godot_api]\nimpl Player {\n    #[func]\n    fn jump(&mut self) {}\n    \
         #[signal]\n    fn hit();\n}\n\
         #[godot_api]\nimpl INode2D for Player {\n    fn ready(&mut self) {}\n    \
         fn process(&mut self, delta: f64) {}\n}\n\
         impl Iterator for Player {\n    type Item = u8;\n    \
         fn next(&mut self) -> Option<u8> { None }\n}\n",
    )]);
    // jump, hit, ready, process; `Iterator` is not an `I*` engine class.
    assert_entries(&err, "gdext", "lifecycle", 4);
}

#[test]
fn bevy_roots_plugin_build_and_registered_systems() {
    let err = run(&[(
        "src/main.rs",
        "use bevy::prelude::*;\n\
         pub struct GamePlugin;\n\
         impl Plugin for GamePlugin {\n    fn build(&self, app: &mut App) {\n        \
         app.add_systems(Startup, setup);\n    }\n}\n\
         fn setup() {}\n",
    )]);
    assert_entries(&err, "bevy", "lifecycle", 2);
}

#[test]
fn rust_ui_components_and_server_functions() {
    let err = run(&[(
        "src/app.rs",
        "use leptos::prelude::*;\n\
         #[component]\npub fn Counter() -> impl IntoView { view! { <p/> } }\n\
         #[server]\npub async fn save(x: i32) -> Result<(), ServerFnError> { Ok(()) }\n",
    )]);
    assert_entries(&err, "rust-ui-components", "lifecycle", 1);
    assert_entries(&err, "rust-server-fn", "network", 1);
}

#[test]
fn kani_harnesses_need_no_import() {
    let err = run(&[(
        "src/lib.rs",
        "pub fn add(a: u8, b: u8) -> u8 { a.wrapping_add(b) }\n\
         #[cfg(kani)]\nmod proofs {\n    #[kani::proof]\n    fn add_commutes() {\n        \
         let a: u8 = kani::any();\n        assert_eq!(super::add(a, 1), super::add(1, a));\n    }\n\
         \x20   #[kani::proof_for_contract(super::add)]\n    fn add_contract() {}\n    \
         #[proof]\n    fn not_kani() {}\n}\n",
    )]);
    assert_entries(&err, "kani", "test", 2);
}

#[test]
fn eframe_app_methods_are_lifecycle_roots() {
    let err = run(&[(
        "src/main.rs",
        "use eframe::egui;\nstruct MyApp;\n\
         impl eframe::App for MyApp {\n    \
         fn update(&mut self, ctx: &egui::Context, f: &mut eframe::Frame) {}\n    \
         fn on_exit(&mut self) {}\n}\n",
    )]);
    assert_entries(&err, "eframe", "lifecycle", 2);
}

#[test]
fn zbus_interface_members_are_entries_but_signals_are_not() {
    let err = run(&[(
        "src/svc.rs",
        "use zbus::{interface, object_server::SignalEmitter};\nstruct Greeter;\n\
         #[interface(name = \"org.x.Greeter\")]\nimpl Greeter {\n    \
         async fn say_hello(&self, name: &str) -> String { name.into() }\n    \
         #[zbus(property)]\n    async fn greeter_name(&self) -> String { String::new() }\n    \
         #[zbus(signal)]\n    async fn greeted(e: &SignalEmitter<'_>) -> zbus::Result<()>;\n}\n",
    )]);
    assert_entries(&err, "zbus", "network", 2);
}

#[test]
fn serenity_poise_teloxide_and_hyper() {
    let err = run(&[
        (
            "src/bot.rs",
            "use serenity::prelude::*;\nuse poise::serenity_prelude as serenity;\n\
             struct Handler;\n\
             #[async_trait]\nimpl EventHandler for Handler {\n    \
             async fn message(&self, ctx: Context, msg: Message) {}\n}\n\
             #[poise::command(slash_command)]\npub async fn age(ctx: Ctx<'_>) -> Res { Ok(()) }\n",
        ),
        (
            "src/tg.rs",
            "use teloxide::prelude::*;\n\
             pub async fn run(bot: Bot) { teloxide::repl(bot, answer).await; }\n\
             async fn answer(bot: Bot, msg: Message) -> ResponseResult<()> { Ok(()) }\n",
        ),
        (
            "src/http.rs",
            "use hyper::service::service_fn;\n\
             pub async fn serve(io: Io) { let _ = conn.serve_connection(io, service_fn(hello)); }\n\
             async fn hello(req: Request) -> Result<Response, Infallible> { todo!() }\n",
        ),
    ]);
    assert_entries(&err, "serenity", "network", 1);
    assert_entries(&err, "poise", "network", 1);
    assert_entries(&err, "teloxide", "network", 1);
    assert_entries(&err, "hyper", "network", 1);
}

// ---------------------------------------------------------------------
// C++
// ---------------------------------------------------------------------

#[test]
fn qt_widgets_event_overrides_and_auto_slots() {
    let err = run(&[(
        "src/win.cpp",
        "#include <QMainWindow>\n\
         class Win : public QMainWindow {\n  void closeEvent(QCloseEvent* e) override {}\n  \
         void on_saveButton_clicked() {}\n  void helper() {}\n};\n",
    )]);
    assert_entries(&err, "qt-widgets", "lifecycle", 2);
}

#[test]
fn qtest_slots_count_only_where_the_harness_runs() {
    let err = run(&[
        (
            "tests/TestGroup.cpp",
            "#include <QTest>\n\
             class TestGroup : public QObject {\nprivate slots:\n  \
             void initTestCase() {}\n  void testAdd() {}\n  void testAdd_data() {}\n};\n\
             QTEST_MAIN(TestGroup)\n",
        ),
        (
            "src/Config.cpp",
            "#include <QObject>\nclass Config : public QObject {\n  void init() {}\n};\n",
        ),
    ]);
    // `Config::init` is production code in a file that does not run QTest.
    assert_entries(&err, "qtest", "test", 3);
}

#[test]
fn boost_test_cases_match_case_sensitively() {
    let err = run(&[(
        "test/t.cc",
        "#include <boost/test/unit_test.hpp>\n\
         BOOST_AUTO_TEST_CASE(test_one) { BOOST_CHECK(true); }\n\
         BOOST_FIXTURE_TEST_CASE(test_two, F) { BOOST_CHECK(true); }\n",
    )]);
    assert_entries(&err, "boost-test", "test", 2);
}

#[test]
fn ros2_lifecycle_and_behaviortree_nodes() {
    let err = run(&[
        (
            "src/node.cpp",
            "#include \"rclcpp_lifecycle/lifecycle_node.hpp\"\n\
             class Amcl : public rclcpp_lifecycle::LifecycleNode {\n  \
             CallbackReturn on_configure(const State&) override { return {}; }\n};\n",
        ),
        (
            "src/bt.cpp",
            "#include \"behaviortree_cpp/action_node.h\"\n\
             class Spin : public BT::SyncActionNode {\n  \
             BT::NodeStatus tick() override { return {}; }\n  \
             static BT::PortsList providedPorts() { return {}; }\n};\n\
             BT_REGISTER_NODES(factory) { factory.registerNodeType<Spin>(\"Spin\"); }\n",
        ),
    ]);
    assert_entries(&err, "ros2-rclcpp", "lifecycle", 1);
    assert_entries(&err, "behaviortree-cpp", "lifecycle", 2);
    assert_entries(&err, "behaviortree-register", "lifecycle", 1);
}

#[test]
fn juce_processor_callbacks_and_plugin_factory() {
    let err = run(&[(
        "Source/PluginProcessor.cpp",
        "#include <JuceHeader.h>\n\
         class Proc : public juce::AudioProcessor {\n  \
         void prepareToPlay(double sr, int n) override {}\n  \
         void processBlock(juce::AudioBuffer<float>& b, juce::MidiBuffer& m) override {}\n};\n\
         juce::AudioProcessor* createPluginFilter() { return new Proc(); }\n",
    )]);
    assert_entries(&err, "juce", "lifecycle", 2);
    assert_entries(&err, "juce-plugin", "ffi", 1);
}

#[test]
fn grpc_cpp_needs_a_qualified_generated_base() {
    let err = run(&[(
        "src/svc.cc",
        "#include <grpcpp/grpcpp.h>\n\
         class PredictionImpl final : public PredictionService::Service {\n  \
         grpc::Status Predict(grpc::ServerContext* c, const Req* r, Resp* o) override { return {}; }\n};\n\
         class Plain : public Service {\n  void Run() {}\n};\n",
    )]);
    assert_entries(&err, "grpc-cpp", "network", 1);
}

#[test]
fn arduino_roots_the_free_setup_and_loop_only() {
    let err = run(&[(
        "src/main.cpp",
        "#include <Arduino.h>\n\
         class GPS { public: void setup(); };\nvoid GPS::setup() {}\n\
         void setup() { Serial.begin(9600); }\nvoid loop() {}\n",
    )]);
    assert_entries(&err, "arduino", "lifecycle", 2);
}

// ---------------------------------------------------------------------
// C
// ---------------------------------------------------------------------

#[test]
fn glib_signal_handlers_through_g_callback_casts_and_address_of() {
    let err = run(&[(
        "src/ui.c",
        "#include <gtk/gtk.h>\n\
         static void on_clicked(GtkWidget *w, gpointer d) {}\n\
         static gboolean tick(gpointer d) { return TRUE; }\n\
         static gboolean idle(gpointer d) { return FALSE; }\n\
         void build(GtkWidget *b) {\n  \
         g_signal_connect(b, \"clicked\", G_CALLBACK(on_clicked), NULL);\n  \
         g_timeout_add(100, (GSourceFunc) tick, NULL);\n  \
         g_idle_add(G_SOURCE_FUNC(idle), NULL);\n}\n",
    )]);
    assert_entries(&err, "glib-signals", "lifecycle", 3);
}

#[test]
fn zephyr_registration_macros() {
    let err = run(&[(
        "src/board.c",
        "#include <zephyr/init.h>\n#include <zephyr/kernel.h>\n\
         static int pinmux_init(void) { return 0; }\n\
         SYS_INIT(pinmux_init, APPLICATION, 50);\n\
         static void worker(void *a, void *b, void *c) {}\n\
         K_THREAD_DEFINE(tid, 512, worker, NULL, NULL, NULL, 5, 0, 0);\n",
    )]);
    assert_entries(&err, "zephyr", "lifecycle", 2);
}

#[test]
fn redis_module_onload_and_commands() {
    let err = run(&[(
        "src/mod.c",
        "#include \"redismodule.h\"\n\
         static int Cmd(RedisModuleCtx *ctx, RedisModuleString **argv, int argc) { return 0; }\n\
         int RedisModule_OnLoad(RedisModuleCtx *ctx, RedisModuleString **argv, int argc) {\n  \
         RedisModule_CreateCommand(ctx, \"bf.add\", Cmd, \"write\", 1, 1, 1);\n  return 0;\n}\n",
    )]);
    assert_entries(&err, "redis-module", "ffi", 2);
}

#[test]
fn sqlite_extension_init_uses_an_infix_pattern() {
    let err = run(&[(
        "vec.c",
        "#include \"sqlite3ext.h\"\nSQLITE_EXTENSION_INIT1\n\
         static void vec_version(sqlite3_context *c, int n, sqlite3_value **v) {}\n\
         int sqlite3_vec_init(sqlite3 *db, char **err, const sqlite3_api_routines *api) {\n  \
         sqlite3_create_function_v2(db, \"vec_version\", 0, 0, 0, vec_version, 0, 0, 0);\n  \
         return 0;\n}\n\
         int sqlite3_helper(void) { return 0; }\n",
    )]);
    // `sqlite3_helper` is not `sqlite3_*_init`.
    assert_entries(&err, "sqlite-ext", "ffi", 2);
}

#[test]
fn esp_idf_and_freertos_entries() {
    let err = run(&[(
        "main/app.c",
        "#include \"esp_event.h\"\n#include \"freertos/FreeRTOS.h\"\n#include \"freertos/task.h\"\n\
         static void on_event(void *a, esp_event_base_t b, int32_t id, void *d) {}\n\
         static void blink(void *p) {}\n\
         void vApplicationIdleHook(void) {}\n\
         void app_main(void) {\n  \
         esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, &on_event, NULL);\n  \
         xTaskCreate(blink, \"blink\", 2048, NULL, 5, NULL);\n}\n",
    )]);
    assert_entries(&err, "esp-idf", "lifecycle", 2);
    assert_entries(&err, "freertos", "lifecycle", 2);
}

#[test]
fn cmsis_handlers_ride_on_the_core_exception_names() {
    let err = run(&[(
        "Src/stm32f4xx_it.c",
        "#include \"main.h\"\n\
         void SysTick_Handler(void) { HAL_IncTick(); }\n\
         void USART2_IRQHandler(void) {}\n\
         void timerHandler(void) {}\n\
         void HAL_UART_RxCpltCallback(void *h) {}\n\
         int main(void) { HAL_Init(); while (1) {} }\n",
    )]);
    assert_entries(&err, "cmsis-vectors", "lifecycle", 2);
    assert_entries(&err, "stm32-hal", "lifecycle", 1);
}

#[test]
fn a_handler_pattern_alone_does_not_detect_cmsis() {
    let err = run(&[(
        "src/it.c",
        "void USART2_IRQHandler(void) {}\nint main(void) { return 0; }\n",
    )]);
    assert!(!err.contains("cmsis-vectors"), "{err}");
}

// ---------------------------------------------------------------------
// Zig, CMake, Nix, Fortran, VHDL
// ---------------------------------------------------------------------

#[test]
fn httpz_and_websocket_hooks() {
    let err = run(&[(
        "src/server.zig",
        "const httpz = @import(\"httpz\");\nconst websocket = @import(\"websocket\");\n\
         const App = struct {\n    \
         pub fn notFound(_: *App, req: *httpz.Request, res: *httpz.Response) !void {}\n    \
         pub fn uncaughtError(_: *App, req: *httpz.Request, res: *httpz.Response, e: anyerror) void {}\n};\n\
         const Client = struct {\n    pub fn clientMessage(self: *Client, data: []const u8) !void {}\n};\n",
    )]);
    assert_entries(&err, "zig-httpz", "network", 2);
    assert_entries(&err, "zig-websocket", "network", 1);
}

#[test]
fn vendored_cmake_apis_are_public_roots() {
    let err = run(&[
        (
            "cmake/CPM.cmake",
            "function(CPMAddPackage)\nendfunction()\nfunction(CPMFindPackage)\nendfunction()\n\
             function(cpm_helper)\nendfunction()\n",
        ),
        (
            "cmake/CodeCoverage.cmake",
            "function(setup_target_for_coverage_lcov)\nendfunction()\n\
             function(append_coverage_compiler_flags)\nendfunction()\n",
        ),
        (
            "cmake/expat-config.cmake",
            "macro(set_and_check _var _file)\nendmacro()\n\
             macro(check_required_components _NAME)\nendmacro()\n",
        ),
    ]);
    assert_entries(&err, "cpm-cmake", "public", 2);
    assert_entries(&err, "cmake-codecoverage", "public", 2);
    assert_entries(&err, "cmake-package-init", "public", 2);
}

#[test]
fn flake_parts_per_system_is_detected_by_mkflake() {
    let err = run(&[(
        "flake.nix",
        "{\n  inputs.flake-parts.url = \"github:hercules-ci/flake-parts\";\n  \
         outputs = inputs: inputs.flake-parts.lib.mkFlake { inherit inputs; } {\n    \
         systems = [ \"x86_64-linux\" ];\n    \
         perSystem = { pkgs, ... }: { packages.default = pkgs.hello; };\n  };\n}\n",
    )]);
    assert_entries(&err, "flake-parts", "lifecycle", 1);
}

#[test]
fn nixos_test_script_and_nodes() {
    let err = run(&[(
        "nix/tests/basic.nix",
        "{ pkgs }:\npkgs.testers.runNixOSTest {\n  name = \"basic\";\n  \
         nodes.machine = { pkgs, ... }: { services.nginx.enable = true; };\n  \
         testScript = { nodes, ... }: ''\n    machine.wait_for_unit(\"nginx\")\n  '';\n}\n",
    )]);
    assert_entries(&err, "nixos-test", "test", 2);
}

#[test]
fn solver_called_fortran_subroutines() {
    let err = run(&[(
        "umat.f90",
        "subroutine umat(stress, statev, ddsdde)\n  real :: stress(6), statev(1), ddsdde(6,6)\n\
         call helper(stress)\nend subroutine umat\n\
         subroutine helper(s)\n  real :: s(6)\nend subroutine helper\n\
         subroutine usermat(x)\n  real :: x\nend subroutine usermat\n\
         subroutine umat41(x)\n  real :: x\nend subroutine umat41\n",
    )]);
    assert_entries(&err, "abaqus-user-subroutine", "ffi", 1);
    assert_entries(&err, "ansys-user-material", "ffi", 1);
    assert_entries(&err, "lsdyna-user-material", "ffi", 1);
}

#[test]
fn esmf_set_services_is_a_component_entry() {
    let err = run(&[(
        "cap.F90",
        "module ocn_cap\n  use ESMF\n  use NUOPC\n  implicit none\ncontains\n\
         subroutine SetServices(gcomp, rc)\n    type(ESMF_GridComp) :: gcomp\n    \
         integer :: rc\n    \
         call NUOPC_CompSpecialize(gcomp, specLabel=label_Advance, specRoutine=ModelAdvance, rc=rc)\n  \
         end subroutine SetServices\n  \
         subroutine ModelAdvance(gcomp, rc)\n    type(ESMF_GridComp) :: gcomp\n    \
         integer :: rc\n    rc = 0\n  end subroutine ModelAdvance\nend module ocn_cap\n",
    )]);
    // `SetServices`, and `ModelAdvance` named by keyword to the registrar.
    assert_entries(&err, "esmf-nuopc", "lifecycle", 2);
}

#[test]
fn vunit_testbenches_by_entity_name() {
    let err = run(&[(
        "tb/tb_adder.vhd",
        "library vunit_lib;\ncontext vunit_lib.vunit_context;\n\
         entity tb_adder is\n  generic (runner_cfg : string);\nend entity;\n\
         architecture tb of tb_adder is\nbegin\nend architecture;\n",
    )]);
    assert_entries(&err, "vunit", "test", 1);
}

#[test]
fn ztest_cases_enter_through_the_function_defining_macro() {
    let err = run(&[(
        "tests/main.c",
        "#include <zephyr/ztest.h>\n\
         static int add(int a, int b) { return a + b; }\n\
         ZTEST(math, test_add) { zassert_equal(add(1, 2), 3); }\n\
         ZTEST_SUITE(math, NULL, NULL, NULL, NULL, NULL);\n",
    )]);
    assert_entries(&err, "ztest", "test", 1);
}

// ---------------------------------------------------------------------
// Rust AI-agent and tool-calling frameworks (0.14.0 Phase C)
// ---------------------------------------------------------------------

#[test]
fn rmcp_hand_written_handlers_split_tools_from_lifecycle() {
    let err = run(&[(
        "src/server.rs",
        "use rmcp::{model::*, service::RequestContext, ErrorData, RoleServer, RoleClient, ServerHandler, ClientHandler};\n\
         pub struct Raw;\n\
         impl ServerHandler for Raw {\n    \
         async fn list_tools(&self, r: Option<P>, c: RequestContext<RoleServer>) -> Result<L, ErrorData> { todo!() }\n    \
         async fn call_tool(&self, r: CallToolRequestParam, c: RequestContext<RoleServer>) -> Result<C, ErrorData> { dispatch(&r.name) }\n    \
         fn get_info(&self) -> ServerInfo { ServerInfo::default() }\n    \
         fn helper(&self) {}\n}\n\
         fn dispatch(n: &str) -> Result<C, ErrorData> { todo!() }\n\
         pub struct Cli;\n\
         impl ClientHandler for Cli {\n    \
         async fn on_logging_message(&self, p: LoggingMessageNotificationParam) {}\n}\n",
    )]);
    // list_tools, call_tool are tools; get_info, on_logging_message are
    // lifecycle; `helper` is neither.
    assert_entries(&err, "rmcp", "tool", 2);
    assert_entries(&err, "rmcp-handlers", "lifecycle", 2);
}

#[test]
fn rig_tools_and_hooks() {
    let err = run(&[(
        "src/main.rs",
        "use rig::tool::{Tool, ToolEmbedding};\nuse rig::agent::PromptHook;\nuse rig_derive::rig_tool;\n\
         struct Adder;\n\
         impl Tool for Adder {\n    const NAME: &'static str = \"add\";\n    \
         async fn definition(&self, p: String) -> ToolDefinition { todo!() }\n    \
         async fn call(&self, a: Args) -> Result<i32, E> { Ok(add_impl(a.x, a.y)) }\n}\n\
         fn add_impl(x: i32, y: i32) -> i32 { x + y }\n\
         #[rig_tool(description = \"Multiply x and y\")]\n\
         fn multiply(x: i32, y: i32) -> Result<i32, rig::tool::ToolError> { Ok(x * y) }\n\
         struct Logger;\n\
         impl<M: CompletionModel> PromptHook<M> for Logger {\n    \
         async fn on_tool_call(&self, name: &str, args: &str) {}\n}\n",
    )]);
    assert_entries(&err, "rig", "tool", 3);
    assert_entries(&err, "rig-hooks", "lifecycle", 1);
}

#[test]
fn a_tool_trait_of_the_apps_own_is_not_rigs() {
    let err = run(&[
        (
            "src/providers.rs",
            "use rig::providers::openai;\npub fn client() -> openai::Client { openai::Client::from_env() }\n",
        ),
        (
            "src/tools.rs",
            "pub trait Tool { fn call(&self); }\nstruct Grep;\n\
             impl Tool for Grep {\n    fn call(&self) {}\n}\n",
        ),
    ]);
    // Rig is detected, but `tools.rs` neither imports it nor names it.
    assert!(!err.contains("rig (tool"), "{err}");
}
