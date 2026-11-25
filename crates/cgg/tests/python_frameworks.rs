//! Python application-framework rules added in 0.14.0 (framework audit
//! 2026-11-25): one fixture per rule, asserting the entry count the
//! coverage report states and, where the rule mints nodes, the node.
//!
//! The fixtures are the audit's, trimmed: each holds the framework's
//! entry shapes plus a lookalike that must not match, so a count that
//! grows means a rule reads too broadly and one that shrinks means it
//! stopped reading a shape.

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

/// Run cgg over the files and return (mermaid graph, stderr).
fn run(files: &[(&str, &str)]) -> (String, String) {
    let tmp = TempDir::new().unwrap();
    for (rel, body) in files {
        write(tmp.path(), rel, body);
    }
    let out = tmp.path().join("g.mmd");
    let assert = Command::cargo_bin("cgg")
        .expect("cgg binary")
        .args(["--jobs", "1", "-o"])
        .arg(&out)
        .arg(tmp.path())
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    (fs::read_to_string(&out).unwrap(), stderr)
}

/// The coverage line for one rule, e.g. `qt (lifecycle, 8 entries)`.
fn assert_entries(err: &str, id: &str, kind: &str, n: usize) {
    let want = if n == 1 {
        format!(" {id} ({kind}, 1 entry)")
    } else {
        format!(" {id} ({kind}, {n} entries)")
    };
    assert!(err.contains(&want), "expected `{want}` in:\n{err}");
}

#[test]
fn blender_addon_callbacks_and_module_register() {
    let (_, err) = run(&[(
        "addon.py",
        "import bpy\n\n\
         class OBJECT_OT_demo(bpy.types.Operator):\n\
         \x20   @classmethod\n\
         \x20   def poll(cls, context):\n        return True\n\
         \x20   def execute(self, context):\n        return {'FINISHED'}\n\
         \x20   def invoke(self, context, event):\n        return self.execute(context)\n\
         \x20   def _helper(self):\n        return 1\n\n\
         class VIEW3D_PT_demo(bpy.types.Panel):\n\
         \x20   def draw(self, context):\n        pass\n\n\
         class Plain:\n\
         \x20   def draw(self):\n        pass\n\n\
         classes = (OBJECT_OT_demo, VIEW3D_PT_demo)\n\n\
         def register():\n    for c in classes:\n        bpy.utils.register_class(c)\n\n\
         def unregister():\n    for c in classes:\n        bpy.utils.unregister_class(c)\n",
    )]);
    // poll, execute, invoke, draw; not `_helper`, not `Plain.draw`.
    assert_entries(&err, "blender-addon", "lifecycle", 4);
    assert_entries(&err, "blender-addon-register", "lifecycle", 2);
}

#[test]
fn qt_slots_connections_and_virtual_overrides() {
    let (_, err) = run(&[(
        "qt_app.py",
        "from PySide6.QtCore import Slot, QThread, QAbstractTableModel\n\
         from PySide6.QtWidgets import QMainWindow, QPushButton\n\n\
         class Worker(QThread):\n    def run(self):\n        pass\n\n\
         class Model(QAbstractTableModel):\n\
         \x20   def rowCount(self, parent=None):\n        return 0\n\
         \x20   def data(self, index, role):\n        return None\n\
         \x20   def rowcount(self):\n        return 0\n\n\
         class Main(QMainWindow):\n\
         \x20   def __init__(self):\n        super().__init__()\n\
         \x20       self.btn = QPushButton('Go')\n\
         \x20       self.btn.clicked.connect(self.on_go)\n\
         \x20   def on_go(self):\n        pass\n\
         \x20   @Slot()\n    def slot_only(self):\n        pass\n\
         \x20   def closeEvent(self, event):\n        pass\n\
         \x20   def keyPressEvent(self, e):\n        pass\n",
    )]);
    // run, rowCount, data, on_go (connect), slot_only, closeEvent,
    // keyPressEvent. `rowcount` is not Qt's `rowCount`.
    assert_entries(&err, "qt", "lifecycle", 7);
}

#[test]
fn textual_handlers_dispatched_by_name() {
    let (_, err) = run(&[(
        "tui.py",
        "from textual.app import App, ComposeResult\n\
         from textual.widgets import Button\n\
         from textual import on, work\n\n\
         class MyApp(App):\n\
         \x20   def compose(self) -> ComposeResult:\n        yield Button('x')\n\
         \x20   def on_mount(self) -> None:\n        pass\n\
         \x20   @on(Button.Pressed, '#b')\n    def handle_b(self) -> None:\n        pass\n\
         \x20   def action_quit_app(self) -> None:\n        pass\n\
         \x20   def watch_count(self, v) -> None:\n        pass\n\
         \x20   @work(thread=True)\n    def load(self) -> None:\n        pass\n\
         \x20   def helper(self):\n        pass\n",
    )]);
    assert_entries(&err, "textual", "lifecycle", 6);
}

#[test]
fn home_assistant_module_hooks_flows_and_entities() {
    let (_, err) = run(&[
        (
            "custom_components/demo/__init__.py",
            "from homeassistant.core import HomeAssistant\n\n\
             async def async_setup_entry(hass: HomeAssistant, entry) -> bool:\n    return True\n\n\
             async def async_unload_entry(hass, entry):\n    return True\n\n\
             def _private(hass):\n    return 1\n",
        ),
        (
            "custom_components/demo/sensor.py",
            "from homeassistant.components.sensor import SensorEntity\n\
             from homeassistant import config_entries\n\n\
             class DemoSensor(SensorEntity):\n\
             \x20   @property\n    def native_value(self):\n        return 1\n\
             \x20   async def async_update(self):\n        pass\n\
             \x20   def _build(self):\n        pass\n\n\
             class DemoFlow(config_entries.ConfigFlow, domain='demo'):\n\
             \x20   async def async_step_user(self, user_input=None):\n        pass\n",
        ),
    ]);
    // Module hooks by name (two setup, one unload) plus the flow step.
    assert_entries(&err, "home-assistant", "lifecycle", 3);
    // native_value, async_update, async_step_user.
    assert_entries(&err, "home-assistant-entity", "lifecycle", 3);
}

#[test]
fn django_receivers_template_tags_forms_and_serializers() {
    let (g, err) = run(&[(
        "hooks.py",
        "from django.db.models.signals import post_save, pre_delete\n\
         from django.dispatch import receiver\n\
         from django import forms, template\n\
         from django.apps import AppConfig\n\
         from rest_framework import serializers\n\n\
         register = template.Library()\n\n\
         @receiver(post_save, sender='x')\n\
         def on_save(sender, instance, **kw):\n    pass\n\n\
         def on_delete(sender, **kw):\n    pass\n\n\
         pre_delete.connect(on_delete)\n\n\
         @register.filter\ndef money(v):\n    return v\n\n\
         @register.simple_tag(takes_context=True)\ndef render_thing(context):\n    pass\n\n\
         class F(forms.Form):\n\
         \x20   def clean_name(self):\n        pass\n\
         \x20   def get_choices(self):\n        pass\n\n\
         class Cfg(AppConfig):\n    def ready(self):\n        pass\n\n\
         class S(serializers.ModelSerializer):\n\
         \x20   def validate_name(self, v):\n        return v\n\
         \x20   def get_total(self, obj):\n        return 1\n\
         \x20   def _helper(self):\n        pass\n",
    )]);
    assert_entries(&err, "django-hooks", "lifecycle", 4);
    assert!(g.contains("django-hooks"), "{g}");
    // clean_name and ready; a form's `get_choices` is not a hook.
    assert_entries(&err, "django-lifecycle", "lifecycle", 2);
    assert_entries(&err, "drf-serializers", "lifecycle", 2);
}

#[test]
fn odoo_decorators_and_model_conventions() {
    let (_, err) = run(&[(
        "models/thing.py",
        "from odoo import api, fields, models\n\n\
         class SaleThing(models.Model):\n\
         \x20   _name = 'sale.thing'\n\
         \x20   total = fields.Float(compute='_compute_total', store=True)\n\
         \x20   @api.depends('line_ids')\n    def _compute_total(self):\n        pass\n\
         \x20   @api.onchange('partner_id')\n    def _onchange_partner(self):\n        pass\n\
         \x20   @api.constrains('total')\n    def _check_total(self):\n        pass\n\
         \x20   @api.model_create_multi\n    def create(self, vals):\n        return super().create(vals)\n\
         \x20   def action_confirm(self):\n        pass\n\
         \x20   def _helper(self):\n        pass\n",
    )]);
    assert_entries(&err, "odoo", "lifecycle", 4);
    // action_confirm is the only one no decorator already entered.
    assert_entries(&err, "odoo-model", "lifecycle", 1);
}

#[test]
fn behave_and_pytest_bdd_steps_are_test_entries() {
    let (g, err) = run(&[(
        "features/steps/steps.py",
        "from behave import given, when, then\n\n\
         @given('a user {name}')\ndef step_user(context, name):\n    pass\n\n\
         @when('they log in')\ndef step_login(context):\n    pass\n\n\
         @then('they see {page}')\ndef step_see(context, page):\n    pass\n",
    )]);
    assert_entries(&err, "behave", "test", 3);
    assert!(g.contains("test::behave"), "{g}");
    assert!(!err.contains("pytest-bdd"), "{err}");

    let (_, err) = run(&[(
        "tests/test_bdd.py",
        "from pytest_bdd import scenarios, given, when, parsers\n\
         scenarios('x.feature')\n\n\
         @given('a cart', target_fixture='cart')\ndef cart():\n    return []\n\n\
         @when(parsers.parse('I add {n:d}'))\ndef add(cart, n):\n    pass\n",
    )]);
    assert_entries(&err, "pytest-bdd", "test", 2);
}

#[test]
fn cocotb_tests_are_test_entries() {
    let (g, err) = run(&[(
        "tb/test_dut.py",
        "import cocotb\nfrom cocotb.triggers import Timer\n\n\
         @cocotb.test()\nasync def test_reset(dut):\n    await Timer(1)\n\n\
         @cocotb.test(skip=False)\nasync def check_counter(dut):\n    pass\n\n\
         async def drive(dut):\n    pass\n",
    )]);
    assert_entries(&err, "cocotb", "test", 2);
    assert!(g.contains("test::cocotb"), "{g}");
}

#[test]
fn temporal_activities_and_workflow_methods() {
    let (g, err) = run(&[(
        "wf.py",
        "from datetime import timedelta\n\
         from temporalio import activity, workflow\n\n\
         @activity.defn\nasync def say_hello(name: str) -> str:\n    return name\n\n\
         @workflow.defn\nclass GreetingWorkflow:\n\
         \x20   @workflow.run\n    async def run(self, name: str) -> str:\n        return name\n\
         \x20   @workflow.signal\n    def finish(self) -> None:\n        pass\n\
         \x20   @workflow.query\n    def status(self) -> str:\n        return 'x'\n\
         \x20   @workflow.update\n    def change(self, v: str) -> str:\n        return v\n\
         \x20   def _private(self):\n        pass\n",
    )]);
    assert_entries(&err, "temporal-python", "queue", 5);
    assert!(g.contains("queue::temporal-python"), "{g}");
}

#[test]
fn grpc_servicer_subclasses_match_by_base_name_pattern() {
    // `*Servicer` is a naming-convention pattern on a base type, which
    // the base-type matcher compared exactly before 0.14.0.
    let (g, err) = run(&[(
        "server.py",
        "import grpc\nimport helloworld_pb2_grpc\n\n\
         class Greeter(helloworld_pb2_grpc.GreeterServicer):\n\
         \x20   def SayHello(self, request, context):\n        return None\n\
         \x20   def SayHelloAgain(self, request, context):\n        return None\n\n\
         class Servicer:\n    def NotAnRpc(self):\n        pass\n\n\
         def serve():\n    server = grpc.server(None)\n\
         \x20   helloworld_pb2_grpc.add_GreeterServicer_to_server(Greeter(), server)\n",
    )]);
    assert_entries(&err, "grpc-python", "network", 2);
    assert!(g.contains("network::grpc-python"), "{g}");
}

#[test]
fn graphene_resolvers_and_mutations() {
    let (_, err) = run(&[(
        "schema.py",
        "import graphene\n\n\
         class Query(graphene.ObjectType):\n\
         \x20   hello = graphene.String()\n\
         \x20   def resolve_hello(root, info):\n        return 'x'\n\
         \x20   def helper(self):\n        return 1\n\n\
         class CreateUser(graphene.Mutation):\n\
         \x20   ok = graphene.Boolean()\n\
         \x20   def mutate(root, info, name):\n        return CreateUser(ok=True)\n",
    )]);
    assert_entries(&err, "graphene", "network", 2);
}

#[test]
fn strawberry_decorated_resolvers() {
    let (_, err) = run(&[(
        "schema.py",
        "import strawberry\nimport strawberry_django\n\n\
         @strawberry.type\nclass Query:\n\
         \x20   @strawberry.field\n    def hello(self) -> str:\n        return 'x'\n\
         \x20   @strawberry_django.field\n    def items(self) -> list[str]:\n        return []\n\n\
         @strawberry.type\nclass Mutation:\n\
         \x20   @strawberry.mutation\n    def add(self, name: str) -> str:\n        return name\n\n\
         @strawberry.type\nclass Subscription:\n\
         \x20   @strawberry.subscription\n    async def count(self) -> int:\n        yield 1\n\n\
         def helper():\n    return 1\n",
    )]);
    assert_entries(&err, "strawberry", "network", 4);
}

#[test]
fn discord_commands_listeners_cog_hooks_and_extension_setup() {
    let (_, err) = run(&[(
        "bot.py",
        "import discord\nfrom discord.ext import commands\n\n\
         bot = commands.Bot(command_prefix='!', intents=discord.Intents.default())\n\n\
         @bot.event\nasync def on_ready():\n    pass\n\n\
         @bot.command()\nasync def ping(ctx):\n    pass\n\n\
         class Music(commands.Cog):\n\
         \x20   @commands.command(name='play')\n    async def play(self, ctx, url):\n        pass\n\
         \x20   @commands.Cog.listener()\n    async def on_member_join(self, member):\n        pass\n\
         \x20   async def cog_load(self):\n        pass\n\
         \x20   def helper(self):\n        return 1\n\n\
         async def setup(bot):\n    await bot.add_cog(Music(bot))\n\n\
         class T:\n    def setUp(self):\n        pass\n",
    )]);
    // on_ready, ping, play, on_member_join, cog_load.
    assert_entries(&err, "discord-py", "network", 5);
    // `setup`, never unittest's `setUp`.
    assert_entries(&err, "discord-py-extension", "lifecycle", 1);
}

#[test]
fn slack_bolt_listeners() {
    let (g, err) = run(&[(
        "app.py",
        "from slack_bolt import App\napp = App(token='x')\n\n\
         @app.command('/hello')\ndef hello(ack, say):\n    ack()\n\n\
         @app.event('app_mention')\ndef mention(event, say):\n    say('yo')\n\n\
         @app.action('button_click')\ndef action(ack):\n    ack()\n\n\
         @app.view('modal_view')\ndef view_submission(ack):\n    ack()\n\n\
         def unused():\n    return 1\n",
    )]);
    assert_entries(&err, "slack-bolt", "network", 4);
    assert!(g.contains("network::slack-bolt::command"), "{g}");
}

#[test]
fn django_ninja_operations() {
    let (_, err) = run(&[(
        "api.py",
        "from ninja import NinjaAPI, Router\napi = NinjaAPI()\nrouter = Router()\n\n\
         @api.get('/hello')\ndef hello(request):\n    return 'hi'\n\n\
         @router.post('/items')\ndef create_item(request, payload: dict):\n    return payload\n\n\
         @api.api_operation(['GET', 'POST'], '/multi')\ndef multi(request):\n    return 1\n\n\
         def unused():\n    return 1\n",
    )]);
    assert_entries(&err, "django-ninja", "network", 3);
}

#[test]
fn mock_patch_decorators_are_not_http_patch_routes() {
    // `@patch(...)`, `@mock.patch(...)` and `@patch.object(...)` used to
    // mint fastapi PATCH routes for the test functions they decorate.
    let (_, err) = run(&[
        (
            "app.py",
            "from fastapi import FastAPI\napp = FastAPI()\n\n\
             @app.patch('/items/{i}')\ndef update_item(i: int):\n    return i\n",
        ),
        (
            "tests/test_app.py",
            "from unittest import mock\nfrom unittest.mock import patch\nimport app\n\n\
             @patch('app.update_item')\ndef test_one(m):\n    assert m\n\n\
             @mock.patch('app.update_item')\ndef test_two(m):\n    assert m\n\n\
             class TestX:\n\
             \x20   @patch.object(app, 'update_item')\n    def test_three(self, m):\n        assert m\n",
        ),
    ]);
    assert_entries(&err, "fastapi", "network", 1);
}
