//! 0.14.0 framework rows for the JVM languages and Clojure.
//!
//! Table-driven: one smallest source per (rule, language) carrying the
//! framework's real hand-off shape, and the coverage line the run must
//! print for it (`<id> (<kind>, <n> entries)`). The count is exact, so a
//! row that starts taking a neighbour (a private helper, a method of an
//! unrelated class) fails here, not in a corpus diff.

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

/// Run cgg over `dir`; return (mermaid graph, stderr).
fn run(dir: &Path, extra: &[&str]) -> (String, String) {
    let out = dir.join("g.mmd");
    let mut cmd = cgg();
    cmd.args(["-o"]).arg(&out).args(extra).arg(dir);
    let assert = cmd.assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

/// The number of dead-code findings at `low` and above.
fn dead_count(dir: &Path) -> usize {
    let (_, err) = run(dir, &["--dead-code", "--dead-code-confidence", "low"]);
    let line = err
        .lines()
        .find(|l| l.contains("callable(s) marked unreferenced"))
        .unwrap_or_else(|| panic!("no dead-code summary:\n{err}"));
    line.split("dead-code: ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("unparsable summary: {line}"))
}

struct Case {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// The coverage line's head, `<id> (<kind>, <n> entr`.
    coverage: &'static str,
    /// A substring the graph must contain (an entry node), or "".
    node: &'static str,
}

const CASES: &[Case] = &[
    // ---- Bukkit / Spigot / Paper ----
    Case {
        name: "bukkit/java",
        files: &[(
            "Main.java",
            "package demo;\nimport org.bukkit.event.EventHandler;\nimport org.bukkit.event.Listener;\nimport org.bukkit.plugin.java.JavaPlugin;\npublic class Main extends JavaPlugin implements Listener {\n  @Override public void onEnable() { helper(); }\n  @EventHandler public void onJoin(Object e) { helper(); }\n  private void helper() {}\n}\n",
        )],
        coverage: "bukkit (lifecycle, 2 entries)",
        node: "",
    },
    Case {
        name: "bukkit/kotlin",
        files: &[(
            "Main.kt",
            "package demo\nimport org.bukkit.event.EventHandler\nimport org.bukkit.event.Listener\nimport org.bukkit.plugin.java.JavaPlugin\nclass Main : JavaPlugin(), Listener {\n  override fun onEnable() { helper() }\n  @EventHandler fun onJoin(e: Any) { helper() }\n  private fun helper() {}\n}\n",
        )],
        coverage: "bukkit (lifecycle, 2 entries)",
        node: "",
    },
    // ---- SpongePowered Mixin ----
    Case {
        name: "mixin/java",
        files: &[(
            "TitleMixin.java",
            "package demo.mixin;\nimport org.spongepowered.asm.mixin.Mixin;\nimport org.spongepowered.asm.mixin.injection.At;\nimport org.spongepowered.asm.mixin.injection.Inject;\nimport org.spongepowered.asm.mixin.injection.Redirect;\nimport com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;\n@Mixin(Object.class)\npublic class TitleMixin {\n  @Inject(method = \"init\", at = @At(\"HEAD\"))\n  private void onInit(Object ci) {}\n  @Redirect(method = \"tick\", at = @At(\"HEAD\"))\n  private int redirectTick(Object o) { return 0; }\n  @WrapOperation(method = \"render\", at = @At(\"HEAD\"))\n  private void wrapRender(Object o) {}\n  private void plain() {}\n}\n",
        )],
        coverage: "mixin (lifecycle, 3 entries)",
        node: "",
    },
    // ---- NeoForge / Forge ----
    Case {
        name: "forge-events/java",
        files: &[(
            "Events.java",
            "package demo;\nimport net.neoforged.bus.api.SubscribeEvent;\npublic class Events {\n  @SubscribeEvent public static void onTick(Object e) {}\n  static void helper() {}\n}\n",
        )],
        coverage: "forge-events (lifecycle, 1 entry)",
        node: "",
    },
    Case {
        name: "forge-events/kotlin",
        files: &[(
            "Events.kt",
            "package demo\nimport net.minecraftforge.eventbus.api.SubscribeEvent\nobject Events {\n  @SubscribeEvent fun onTick(e: Any) {}\n  fun helper() {}\n}\n",
        )],
        coverage: "forge-events (lifecycle, 1 entry)",
        node: "",
    },
    // ---- libGDX ----
    Case {
        name: "libgdx/java",
        files: &[(
            "MyGame.java",
            "package demo;\nimport com.badlogic.gdx.ApplicationAdapter;\nimport com.badlogic.gdx.ScreenAdapter;\npublic class MyGame extends ApplicationAdapter {\n  @Override public void create() {}\n  @Override public void render() {}\n  void helper() {}\n}\nclass Menu extends ScreenAdapter {\n  @Override public void show() {}\n}\n",
        )],
        coverage: "libgdx (lifecycle, 3 entries)",
        node: "",
    },
    Case {
        name: "libgdx/kotlin",
        files: &[(
            "Game.kt",
            "package demo\nimport ktx.app.KtxGame\nimport ktx.app.KtxScreen\nclass Game : KtxGame<KtxScreen>() {\n  override fun create() {}\n  fun helper() {}\n}\nclass Menu : KtxScreen {\n  override fun render(delta: Float) {}\n}\n",
        )],
        coverage: "libgdx (lifecycle, 2 entries)",
        node: "",
    },
    // ---- JDA ----
    Case {
        name: "jda/java",
        files: &[(
            "Bot.java",
            "package demo;\nimport net.dv8tion.jda.api.hooks.ListenerAdapter;\nimport net.dv8tion.jda.api.hooks.SubscribeEvent;\npublic class Bot extends ListenerAdapter {\n  @Override public void onMessageReceived(Object e) { reply(); }\n  private void reply() {}\n}\nclass Other {\n  @SubscribeEvent public void ready(Object e) {}\n}\n",
        )],
        coverage: "jda (network, 2 entries)",
        node: "network::jda::",
    },
    Case {
        name: "jda/kotlin",
        files: &[(
            "Bot.kt",
            "package demo\nimport net.dv8tion.jda.api.hooks.ListenerAdapter\nclass Bot : ListenerAdapter() {\n  override fun onSlashCommandInteraction(e: Any) { reply() }\n  private fun reply() {}\n}\n",
        )],
        coverage: "jda (network, 1 entry)",
        node: "network::jda::",
    },
    // ---- Netty ----
    Case {
        name: "netty/java",
        files: &[(
            "Server.java",
            "package demo;\nimport io.netty.channel.ChannelInitializer;\nimport io.netty.channel.SimpleChannelInboundHandler;\nclass Init extends ChannelInitializer<Object> {\n  @Override protected void initChannel(Object ch) {}\n}\nclass Handler extends SimpleChannelInboundHandler<String> {\n  @Override protected void channelRead0(Object ctx, String msg) { log(); }\n  @Override public void exceptionCaught(Object ctx, Throwable t) {}\n  private void log() {}\n}\n",
        )],
        coverage: "netty (network, 3 entries)",
        node: "network::netty::",
    },
    Case {
        name: "netty/kotlin",
        files: &[(
            "Server.kt",
            "package demo\nimport io.netty.channel.SimpleChannelInboundHandler\nclass Handler : SimpleChannelInboundHandler<String>() {\n  override fun channelRead0(ctx: Any, msg: String) { log() }\n  private fun log() {}\n}\n",
        )],
        coverage: "netty (network, 1 entry)",
        node: "network::netty::",
    },
    // ---- Quartz ----
    Case {
        name: "quartz/java",
        files: &[(
            "Jobs.java",
            "package demo;\nimport org.quartz.Job;\nimport org.quartz.JobExecutionContext;\nimport org.springframework.scheduling.quartz.QuartzJobBean;\npublic class Cleanup implements Job {\n  public void execute(JobExecutionContext c) { run(); }\n  private void run() {}\n}\nclass Report extends QuartzJobBean {\n  protected void executeInternal(JobExecutionContext c) {}\n}\n",
        )],
        coverage: "quartz (schedule, 2 entries)",
        node: "schedule::quartz::",
    },
    Case {
        name: "quartz/kotlin",
        files: &[(
            "Jobs.kt",
            "package demo\nimport org.quartz.Job\nimport org.quartz.JobExecutionContext\nclass Cleanup : Job {\n  override fun execute(c: JobExecutionContext) { run() }\n  private fun run() {}\n}\n",
        )],
        coverage: "quartz (schedule, 1 entry)",
        node: "schedule::quartz::",
    },
    // ---- BPMN delegates ----
    Case {
        name: "bpmn-delegates/java",
        files: &[(
            "Delegates.java",
            "package demo;\nimport org.flowable.engine.delegate.JavaDelegate;\nimport org.flowable.engine.delegate.TaskListener;\npublic class Approve implements JavaDelegate {\n  public void execute(Object execution) {}\n}\nclass Notify implements TaskListener {\n  public void notify(Object task) {}\n}\n",
        )],
        coverage: "bpmn-delegates (queue, 2 entries)",
        node: "queue::bpmn-delegates::",
    },
    Case {
        name: "bpmn-delegates/kotlin",
        files: &[(
            "Delegates.kt",
            "package demo\nimport org.camunda.bpm.engine.delegate.JavaDelegate\nclass Approve : JavaDelegate {\n  override fun execute(execution: Any) {}\n}\n",
        )],
        coverage: "bpmn-delegates (queue, 1 entry)",
        node: "queue::bpmn-delegates::",
    },
    // ---- RocketMQ ----
    Case {
        name: "rocketmq/java",
        files: &[(
            "Consumer.java",
            "package demo;\nimport org.apache.rocketmq.spring.core.RocketMQListener;\npublic class Consumer implements RocketMQListener<String> {\n  public void onMessage(String m) {}\n}\n",
        )],
        coverage: "rocketmq (queue, 1 entry)",
        node: "queue::rocketmq::",
    },
    Case {
        name: "rocketmq/kotlin",
        files: &[(
            "Consumer.kt",
            "package demo\nimport org.apache.rocketmq.spring.core.RocketMQListener\nclass Consumer : RocketMQListener<String> {\n  override fun onMessage(m: String) {}\n}\n",
        )],
        coverage: "rocketmq (queue, 1 entry)",
        node: "queue::rocketmq::",
    },
    // ---- XXL-JOB ----
    Case {
        name: "xxl-job/java",
        files: &[(
            "Jobs.java",
            "package demo;\nimport com.xxl.job.core.handler.annotation.XxlJob;\npublic class Jobs {\n  @XxlJob(\"sync\")\n  public void sync() {}\n  public void helper() {}\n}\n",
        )],
        coverage: "xxl-job (schedule, 1 entry)",
        node: "schedule::xxl-job::",
    },
    Case {
        name: "xxl-job/kotlin",
        files: &[(
            "Jobs.kt",
            "package demo\nimport com.xxl.job.core.handler.annotation.XxlJob\nclass Jobs {\n  @XxlJob(\"sync\")\n  fun sync() {}\n}\n",
        )],
        coverage: "xxl-job (schedule, 1 entry)",
        node: "schedule::xxl-job::",
    },
    // ---- Spring WebSocket ----
    Case {
        name: "spring-websocket/java",
        files: &[(
            "Ws.java",
            "package demo;\nimport org.springframework.web.socket.handler.TextWebSocketHandler;\npublic class Ws extends TextWebSocketHandler {\n  @Override protected void handleTextMessage(Object s, Object m) {}\n  @Override public void afterConnectionEstablished(Object s) {}\n}\n",
        )],
        coverage: "spring-websocket (network, 2 entries)",
        node: "network::spring-websocket::",
    },
    Case {
        name: "spring-websocket/kotlin",
        files: &[(
            "Ws.kt",
            "package demo\nimport org.springframework.web.socket.handler.TextWebSocketHandler\nclass Ws : TextWebSocketHandler() {\n  override fun handleTextMessage(s: Any, m: Any) {}\n}\n",
        )],
        coverage: "spring-websocket (network, 1 entry)",
        node: "network::spring-websocket::",
    },
    // ---- Netflix DGS ----
    Case {
        name: "dgs/java",
        files: &[(
            "Shows.java",
            "package demo;\nimport com.netflix.graphql.dgs.DgsComponent;\nimport com.netflix.graphql.dgs.DgsQuery;\nimport com.netflix.graphql.dgs.DgsMutation;\n@DgsComponent\npublic class Shows {\n  @DgsQuery public Object shows() { return null; }\n  @DgsMutation public Object addShow() { return null; }\n}\n",
        )],
        coverage: "dgs (network, 2 entries)",
        node: "network::dgs::",
    },
    Case {
        name: "dgs/kotlin",
        files: &[(
            "Shows.kt",
            "package demo\nimport com.netflix.graphql.dgs.DgsData\nclass Shows {\n  @DgsData(parentType = \"Query\", field = \"shows\")\n  fun shows(): Any? = null\n}\n",
        )],
        coverage: "dgs (network, 1 entry)",
        node: "network::dgs::",
    },
    // ---- Temporal ----
    Case {
        name: "temporal/java",
        files: &[(
            "Flow.java",
            "package demo;\nimport io.temporal.workflow.WorkflowInterface;\nimport io.temporal.workflow.WorkflowMethod;\nimport io.temporal.workflow.SignalMethod;\n@WorkflowInterface\npublic interface Flow {\n  @WorkflowMethod String run(String in);\n  @SignalMethod void cancel();\n}\n",
        )],
        coverage: "temporal (queue, 2 entries)",
        node: "queue::temporal::",
    },
    Case {
        name: "temporal/kotlin",
        files: &[(
            "Acts.kt",
            "package demo\nimport io.temporal.activity.ActivityInterface\nimport io.temporal.activity.ActivityMethod\n@ActivityInterface\ninterface Acts {\n  @ActivityMethod fun fetch(id: String): String\n}\n",
        )],
        coverage: "temporal (queue, 1 entry)",
        node: "queue::temporal::",
    },
    // ---- gRPC (base-type name patterns) ----
    Case {
        name: "grpc-java/java",
        files: &[(
            "QueryService.java",
            "package demo;\nimport io.grpc.stub.StreamObserver;\npublic class QueryService extends QueryGrpc.QueryImplBase {\n  @Override public void query(Object req, StreamObserver<Object> out) {}\n}\nclass Plain extends Base {\n  public void query(Object req) {}\n}\n",
        )],
        coverage: "grpc-java (network, 1 entry)",
        node: "network::grpc-java::",
    },
    Case {
        name: "grpc-kotlin/kotlin",
        files: &[(
            "Greeter.kt",
            "package demo\nimport io.grpc.Status\nclass Greeter : GreeterGrpcKt.GreeterCoroutineImplBase() {\n  override suspend fun sayHello(request: Any): Any = request\n}\n",
        )],
        coverage: "grpc-kotlin (network, 1 entry)",
        node: "network::grpc-kotlin::",
    },
    // ---- Fault-tolerance fallbacks ----
    Case {
        name: "fault-tolerance/java",
        files: &[(
            "Svc.java",
            "package demo;\nimport io.github.resilience4j.circuitbreaker.annotation.CircuitBreaker;\nimport org.springframework.retry.annotation.Recover;\npublic class Svc {\n  @CircuitBreaker(name = \"backend\", fallbackMethod = \"fallback\")\n  public String call() { return \"x\"; }\n  private String fallback(Throwable t) { return \"y\"; }\n  @Recover\n  public String recover(Exception e) { return \"z\"; }\n}\n",
        )],
        coverage: "fault-tolerance (lifecycle, 2 entries)",
        node: "",
    },
    Case {
        name: "fault-tolerance/kotlin",
        files: &[(
            "Svc.kt",
            "package demo\nimport com.alibaba.csp.sentinel.annotation.SentinelResource\nclass Svc {\n  @SentinelResource(value = \"res\", blockHandler = \"onBlock\")\n  fun call(): String = \"x\"\n  fun onBlock(e: Exception): String = \"y\"\n  fun res(): String = \"z\"\n}\n",
        )],
        coverage: "fault-tolerance (lifecycle, 1 entry)",
        node: "",
    },
    // ---- IntelliJ Platform ----
    Case {
        name: "intellij-platform/kotlin",
        files: &[(
            "Act.kt",
            "package demo\nimport com.intellij.openapi.actionSystem.AnAction\nimport com.intellij.openapi.actionSystem.AnActionEvent\nimport com.intellij.openapi.wm.ToolWindowFactory\nclass Act : AnAction() {\n  override fun actionPerformed(e: AnActionEvent) { helper() }\n  override fun update(e: AnActionEvent) {}\n  private fun helper() {}\n}\nclass Win : ToolWindowFactory {\n  override fun createToolWindowContent(p: Any, w: Any) {}\n}\n",
        )],
        coverage: "intellij-platform (lifecycle, 3 entries)",
        node: "",
    },
    Case {
        name: "intellij-platform/java",
        files: &[(
            "Act.java",
            "package demo;\nimport com.intellij.openapi.actionSystem.AnAction;\npublic class Act extends AnAction {\n  @Override public void actionPerformed(Object e) {}\n}\n",
        )],
        coverage: "intellij-platform (lifecycle, 1 entry)",
        node: "",
    },
    // ---- AndroidX Paging / Room / kotlinx.serialization ----
    Case {
        name: "androidx-paging/kotlin",
        files: &[(
            "Src.kt",
            "package demo\nimport androidx.paging.PagingSource\nimport androidx.paging.PagingState\nclass Src : PagingSource<Int, String>() {\n  override suspend fun load(params: LoadParams<Int>): LoadResult<Int, String> = TODO()\n  override fun getRefreshKey(state: PagingState<Int, String>): Int? = null\n}\n",
        )],
        coverage: "androidx-paging (lifecycle, 2 entries)",
        node: "",
    },
    Case {
        name: "androidx-paging/java",
        files: &[(
            "Src.java",
            "package demo;\nimport androidx.paging.rxjava3.RxPagingSource;\npublic class Src extends RxPagingSource<Integer, String> {\n  public Object loadSingle(Object params) { return null; }\n}\n",
        )],
        coverage: "androidx-paging (lifecycle, 1 entry)",
        node: "",
    },
    Case {
        name: "room/kotlin",
        files: &[(
            "Db.kt",
            "package demo\nimport androidx.room.TypeConverter\nimport androidx.room.RoomDatabase\nimport androidx.room.migration.Migration\nclass Converters {\n  @TypeConverter fun toDate(v: Long): Any = v\n}\nclass M1 : Migration(1, 2) {\n  override fun migrate(db: Any) {}\n}\nclass Cb : RoomDatabase.Callback() {\n  override fun onCreate(db: Any) {}\n}\n",
        )],
        coverage: "room (lifecycle, 3 entries)",
        node: "",
    },
    Case {
        name: "room/java",
        files: &[(
            "Converters.java",
            "package demo;\nimport androidx.room.TypeConverter;\npublic class Converters {\n  @TypeConverter public static Long fromDate(Object d) { return 0L; }\n}\n",
        )],
        coverage: "room (lifecycle, 1 entry)",
        node: "",
    },
    Case {
        name: "kotlinx-serialization/kotlin",
        files: &[(
            "Ser.kt",
            "package demo\nimport kotlinx.serialization.KSerializer\nimport kotlinx.serialization.encoding.Decoder\nimport kotlinx.serialization.encoding.Encoder\nobject DateSer : KSerializer<Long> {\n  override fun serialize(encoder: Encoder, value: Long) {}\n  override fun deserialize(decoder: Decoder): Long = 0\n}\n",
        )],
        coverage: "kotlinx-serialization (lifecycle, 2 entries)",
        node: "",
    },
    // ---- Flink ----
    Case {
        name: "flink/java",
        files: &[(
            "Job.java",
            "package demo;\nimport org.apache.flink.api.common.functions.RichMapFunction;\npublic class Upper extends RichMapFunction<String, String> {\n  @Override public String map(String s) { return norm(s); }\n  private String norm(String s) { return s; }\n}\n",
        )],
        coverage: "flink (queue, 1 entry)",
        node: "queue::flink::",
    },
    Case {
        name: "flink/scala",
        files: &[(
            "Job.scala",
            "package demo\nimport org.apache.flink.streaming.api.functions.KeyedProcessFunction\nclass Dedup extends KeyedProcessFunction[String, String, String] {\n  override def processElement(v: String, ctx: Any, out: Any): Unit = ()\n  override def onTimer(t: Long, ctx: Any, out: Any): Unit = ()\n}\n",
        )],
        coverage: "flink (queue, 2 entries)",
        node: "queue::flink::",
    },
    Case {
        name: "flink/kotlin",
        files: &[(
            "Job.kt",
            "package demo\nimport org.apache.flink.api.common.functions.MapFunction\nclass Upper : MapFunction<String, String> {\n  override fun map(s: String): String = s\n}\n",
        )],
        coverage: "flink (queue, 1 entry)",
        node: "queue::flink::",
    },
    // ---- Scala ----
    Case {
        name: "zio-test/scala",
        files: &[(
            "src/test/scala/demo/MySpec.scala",
            "package demo\nimport zio.test._\nobject MySpec extends ZIOSpecDefault {\n  def spec = suite(\"s\")(test(\"t\")(assertTrue(true)))\n}\n",
        )],
        coverage: "zio-test (test, 1 entry)",
        node: "test::zio-test",
    },
    Case {
        name: "slick/scala",
        files: &[(
            "Tables.scala",
            "package demo\nimport slick.jdbc.H2Profile.api._\nclass Users(tag: Tag) extends Table[User](tag, \"USERS\") {\n  def id = column[Int](\"ID\")\n  def name = column[String](\"NAME\")\n  def * = (id, name)\n}\n",
        )],
        coverage: "slick (lifecycle, 4 entries)",
        node: "",
    },
    Case {
        name: "scalajs-export/scala",
        files: &[(
            "Main.scala",
            "package demo\nimport scala.scalajs.js.annotation._\nobject Api {\n  @JSExportTopLevel(\"render\")\n  def render(): Unit = ()\n  def helper(): Unit = ()\n}\n",
        )],
        coverage: "scalajs-export (ffi, 1 entry)",
        node: "ffi::scalajs-export::",
    },
    Case {
        name: "scala-main/scala",
        files: &[(
            "Migrate.scala",
            "package demo\n@main def migrate(args: String*): Unit = run()\ndef run(): Unit = ()\n",
        )],
        coverage: "scala-main (cli, 1 entry)",
        node: "cli::scala-main::",
    },
    Case {
        name: "kotlin-js-export/kotlin",
        files: &[(
            "Api.kt",
            "package demo\n@JsExport\nfun render(): String = helper()\nfun helper(): String = \"x\"\n",
        )],
        coverage: "kotlin-js-export (ffi, 1 entry)",
        node: "ffi::kotlin-js-export::",
    },
    // ---- Groovy ----
    Case {
        name: "grails-quartz/groovy",
        files: &[(
            "grails-app/jobs/demo/CleanupJob.groovy",
            "package demo\nimport org.quartz.JobExecutionContext\nclass CleanupJob {\n  static triggers = { simple repeatInterval: 5000l }\n  def execute(JobExecutionContext ctx) { purge() }\n  def purge() {}\n}\n",
        )],
        coverage: "grails-quartz (schedule, 1 entry)",
        node: "schedule::grails-quartz::",
    },
    Case {
        name: "hubitat/groovy",
        files: &[(
            "drivers/Dimmer.groovy",
            "metadata {\n  definition(name: \"Dimmer\", namespace: \"demo\", author: \"x\") {\n    capability \"Switch\"\n  }\n}\ndef installed() { initialize() }\ndef updated() { initialize() }\ndef initialize() {}\ndef parse(String description) {}\ndef on() { sendEvent(name: \"switch\", value: \"on\") }\ndef off() { sendEvent(name: \"switch\", value: \"off\") }\n",
        )],
        coverage: "hubitat (lifecycle, 5 entries)",
        node: "",
    },
    // ---- Clojure ----
    Case {
        name: "re-frame/clojure",
        files: &[(
            "src/app/events.cljs",
            "(ns app.events (:require [re-frame.core :as rf]))\n(defn items-sub [db _] (:items db))\n(rf/reg-sub ::items items-sub)\n(rf/reg-event-db ::save\n  (fn [db [_ v]] (assoc db :v v)))\n(rf/reg-event-fx :load [rf/trim-v]\n  (fn [{:keys [db]} _] {:db db}))\n",
        )],
        coverage: "re-frame (lifecycle, 3 entries)",
        node: "lifecycle::re-frame::reg-event-db('::save')",
    },
    Case {
        name: "cljs-export/clojure",
        files: &[(
            "src/app/core.cljs",
            "(ns app.core)\n(defn mount-root [] 1)\n(defn ^:export init! [] (mount-root))\n(defn ^:dev/after-load reload! [] (mount-root))\n",
        )],
        coverage: "cljs-export (ffi, 1 entry)",
        node: "ffi::cljs-export::",
    },
    Case {
        name: "shadow-cljs-hooks/clojure",
        files: &[(
            "src/app/core.cljs",
            "(ns app.core)\n(defn mount-root [] 1)\n(defn ^:dev/after-load reload! [] (mount-root))\n",
        )],
        coverage: "shadow-cljs-hooks (lifecycle, 1 entry)",
        node: "",
    },
];

#[test]
fn every_jvm_and_clojure_row_enumerates_its_fixture() {
    let mut failures = Vec::new();
    for case in CASES {
        let tmp = TempDir::new().unwrap();
        for (rel, src) in case.files {
            write(tmp.path(), rel, src);
        }
        let (g, err) = run(tmp.path(), &[]);
        if !err.contains(case.coverage) {
            failures.push(format!(
                "{}: expected `{}` in the coverage table:\n{err}",
                case.name, case.coverage
            ));
            continue;
        }
        if !case.node.is_empty() && !g.contains(case.node) {
            failures.push(format!(
                "{}: expected an entry node `{}`:\n{g}",
                case.name, case.node
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Without the framework's import (or marker) the same names are not
/// entries — what makes the counts above evidence rather than tautology.
#[test]
fn the_rows_need_their_framework() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Plain.java",
        "package demo;\npublic class Plain extends QueryImplBase {\n  public void onEnable() {}\n  public void execute() {}\n  public void channelRead0() {}\n}\n",
    );
    write(
        tmp.path(),
        "Plain.kt",
        "package demo\nclass Plain : Job {\n  fun execute() {}\n  fun onMessage() {}\n}\n",
    );
    write(
        tmp.path(),
        "plain.clj",
        "(ns plain)\n(defn reg-sub [k f] f)\n(reg-sub :k (fn [] 1))\n",
    );
    let (_, err) = run(tmp.path(), &[]);
    for id in [
        "bukkit",
        "quartz",
        "netty",
        "grpc-java",
        "grpc-kotlin",
        "rocketmq",
        "re-frame",
    ] {
        assert!(
            !err.contains(&format!("{id} (")),
            "`{id}` fired without its framework:\n{err}"
        );
    }
}

/// A fallback is called by the library by name; before the row it was a
/// `high` dead finding.
#[test]
fn a_named_fallback_is_no_longer_dead() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Svc.java",
        "package demo;\nimport io.github.resilience4j.circuitbreaker.annotation.CircuitBreaker;\npublic class Svc {\n  @CircuitBreaker(name = \"backend\", fallbackMethod = \"fallback\")\n  public String call() { return \"x\"; }\n  private String fallback(Throwable t) { return \"y\"; }\n  private String unused() { return \"z\"; }\n}\n",
    );
    // `call` is public API (an untyped root in a library), `unused` is the
    // one real finding.
    let (_, err) = run(
        tmp.path(),
        &["--dead-code", "--dead-code-confidence", "low"],
    );
    assert_eq!(dead_count(tmp.path()), 1, "{err}");
}

/// A Mixin injector is private and never called from source: before the
/// row each one was a `high` finding.
#[test]
fn mixin_injectors_are_not_dead() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "M.java",
        "package demo;\nimport org.spongepowered.asm.mixin.Mixin;\nimport org.spongepowered.asm.mixin.injection.Inject;\n@Mixin(Object.class)\nclass M {\n  @Inject(method = \"x\", at = @At(\"HEAD\"))\n  private void hook(Object ci) { helper(); }\n  private void helper() {}\n  private void unused() {}\n}\n",
    );
    assert_eq!(dead_count(tmp.path()), 1);
}

/// re-frame's inline handlers become callables whose callees are live,
/// and `^:export` / reload hooks keep their call trees live.
#[test]
fn clojurescript_entries_keep_their_call_trees_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/app/core.cljs",
        "(ns app.core (:require [re-frame.core :as rf]))\n\
         (defn save-to [db v] (assoc db :v v))\n\
         (rf/reg-event-db ::save (fn [db [_ v]] (save-to db v)))\n\
         (defn mount-root [] 1)\n\
         (defn ^:export init! [] (mount-root))\n\
         (defn ^:dev/after-load reload! [] (mount-root))\n\
         (defn- unused [] 2)\n",
    );
    assert_eq!(dead_count(tmp.path()), 1);
}
