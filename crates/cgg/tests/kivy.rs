//! KV files are parsed and their `on_*` calls resolve into Python methods.

use assert_cmd::Command;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

fn write(dir: &Path, name: &str, body: &[u8]) {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::File::create(&p).unwrap().write_all(body).unwrap();
}

#[test]
fn kv_file_is_analyzed_not_skipped() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "FacingWizardPopup.kv",
        b"<FacingWizardPopup>:\n    Button:\n        on_release: root.generate_program()\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(
        g.contains("generate_program") || g.contains("on_release"),
        "expected a KV binding node, got:\n{g}"
    );
}

#[test]
fn kv_on_release_resolves_to_python_method() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "popup.py",
        b"class FacingWizardPopup:\n    def generate_program(self):\n        return 1\n    def dismiss(self):\n        return 0\n",
    );
    write(
        tmp.path(),
        "controller.py",
        b"class Controller:\n    def estopCommand(self):\n        return None\n",
    );
    write(
        tmp.path(),
        "FacingWizardPopup.kv",
        b"<FacingWizardPopup>:\n    Button:\n        on_release: root.generate_program()\n        on_press: app.root.controller.estopCommand()\n",
    );

    let mmd = tmp.path().join("g.mmd");
    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(
        g.contains("generate_program"),
        "python method missing from graph:\n{g}"
    );
    assert!(
        g.contains("estopCommand"),
        "controller method missing from graph:\n{g}"
    );
    assert!(
        g.contains("-->|ffi|"),
        "expected a kv→python ffi edge, got:\n{g}"
    );
}

#[test]
fn kv_binding_keeps_python_method_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "popup.py",
        b"class FacingWizardPopup:\n    def generate_program(self):\n        return 1\n    def unused(self):\n        return 0\n",
    );
    write(
        tmp.path(),
        "FacingWizardPopup.kv",
        b"<FacingWizardPopup>:\n    Button:\n        on_release: root.generate_program()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"generate_program"),
        "generate_program should be live via KV, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused"),
        "unused should still be reported, findings: {names:?}"
    );
}

#[test]
fn kv_indented_on_release_keeps_python_methods_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "popup.py",
        b"class WCSSettingsPopup:\n    def apply_changes(self):\n        return 1\n    def dismiss(self):\n        return 0\n    def unused(self):\n        return 2\n",
    );
    write(
        tmp.path(),
        "WCSSettingsPopup.kv",
        b"<WCSSettingsPopup>:\n    Button:\n        on_release:\n            root.apply_changes()\n            root.dismiss()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"apply_changes"),
        "apply_changes should be live via indented KV, findings: {names:?}"
    );
    assert!(
        !names.contains(&"dismiss"),
        "dismiss should be live via indented KV, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused"),
        "unused should still be reported, findings: {names:?}"
    );
}

#[test]
fn kv_app_root_resolves_to_build_return_type() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        b"from kivy.app import App\n\
class Makera:\n    def run_macro(self, n):\n        return n\n    def unused(self):\n        return 0\n\
class Other:\n    def run_macro(self, n):\n        return n\n\
class MakeraApp(App):\n    def build(self):\n        return Makera()\n",
    );
    write(
        tmp.path(),
        "makera.kv",
        b"Button:\n    on_release: app.root.run_macro(1)\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let findings = parsed["findings"].as_array().unwrap();
    let qns: Vec<&str> = findings
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("Makera.run_macro")),
        "Makera.run_macro should be live via app.root, findings: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("Makera.unused")),
        "Makera.unused should still be reported, findings: {qns:?}"
    );
}

#[test]
fn kivy_bind_and_app_build_are_framework_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ui.py",
        b"from kivy.app import App\nfrom kivy.uix.popup import Popup\n\
class P(Popup):\n    def __init__(self):\n        self.bind(on_release=self.open_popup)\n    def open_popup(self):\n        return 1\n    def unused(self):\n        return 0\n\
class A(App):\n    def build(self):\n        return P()\n    def on_start(self):\n        pass\n    def dead(self):\n        return 2\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"open_popup"),
        "open_popup should be live via bind(), findings: {names:?}"
    );
    assert!(
        !names.contains(&"build"),
        "App.build should be a kivy lifecycle root, findings: {names:?}"
    );
    assert!(
        !names.contains(&"on_start"),
        "App.on_start should be a kivy lifecycle root, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused") || names.contains(&"dead"),
        "unrelated methods should still be reported, findings: {names:?}"
    );
}

#[test]
fn kivy_property_observer_is_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "label.py",
        b"from kivy.uix.widget import Widget\nfrom kivy.properties import StringProperty\n\
class L(Widget):\n    text = StringProperty('')\n    def on_text(self, instance, value):\n        return 1\n    def unused(self):\n        return 0\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"on_text"),
        "on_text should be live as a kivy observer, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused"),
        "unused should still be reported, findings: {names:?}"
    );
}

#[test]
fn kv_statement_if_keeps_python_methods_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app.py",
        b"from kivy.app import App\n\
class Makera:\n    def play(self, n, start=None):\n        return n\n    def apply(self):\n        return 1\n    def unused(self):\n        return 0\n\
class MakeraApp(App):\n    def build(self):\n        return Makera()\n",
    );
    write(
        tmp.path(),
        "makera.kv",
        b"Button:\n    on_release:\n        if root.mode == 'Run': app.root.play(1)\n        else: app.root.apply()\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let qns: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["qualified_name"].as_str())
        .collect();
    assert!(
        !qns.iter().any(|q| q.contains("Makera.play")),
        "Makera.play should be live via statement-if KV, findings: {qns:?}"
    );
    assert!(
        !qns.iter().any(|q| q.contains("Makera.apply")),
        "Makera.apply should be live via else: KV, findings: {qns:?}"
    );
    assert!(
        qns.iter().any(|q| q.contains("Makera.unused")),
        "Makera.unused should still be reported, findings: {qns:?}"
    );
}

#[test]
fn kivy_bind_prefers_same_class_when_handler_names_collide() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "widgets.py",
        b"from kivy.uix.widget import Widget\nfrom kivy.core.window import Window\n\
class A(Widget):\n    def __init__(self, **kwargs):\n        super().__init__(**kwargs)\n        Window.bind(mouse_pos=self.on_mouse_pos)\n    def on_mouse_pos(self, *args):\n        return 1\n    def unused_a(self):\n        return 0\n\
class B(Widget):\n    def __init__(self, **kwargs):\n        super().__init__(**kwargs)\n        Window.bind(mouse_pos=self.on_mouse_pos)\n    def on_mouse_pos(self, *args):\n        return 2\n    def unused_b(self):\n        return 0\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"on_mouse_pos"),
        "both on_mouse_pos handlers should be live via same-class bind, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused_a") || names.contains(&"unused_b"),
        "unrelated methods should still be reported, findings: {names:?}"
    );
}

#[test]
fn kivy_slider_and_settingitem_lifecycle_methods_are_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "ui.py",
        b"from kivy.uix.slider import Slider\nfrom kivy.uix.settings import SettingItem\nfrom kivy.uix.dropdown import DropDown\n\
class S(Slider):\n    def on_value(self, instance, value):\n        return 1\n    def unused(self):\n        return 0\n\
class I(SettingItem):\n    def on_parent(self, instance, value):\n        return 1\n\
class D(DropDown):\n    def on_dismiss(self):\n        return 1\n",
    );

    let report = tmp.path().join("dead.json");
    cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-report",
        ])
        .arg(&report)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&report).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = parsed["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["simple_name"].as_str())
        .collect();
    assert!(
        !names.contains(&"on_value"),
        "Slider.on_value should be a kivy lifecycle root, findings: {names:?}"
    );
    assert!(
        !names.contains(&"on_parent"),
        "SettingItem.on_parent should be a kivy lifecycle root, findings: {names:?}"
    );
    assert!(
        !names.contains(&"on_dismiss"),
        "DropDown.on_dismiss should be a kivy lifecycle root, findings: {names:?}"
    );
    assert!(
        names.contains(&"unused"),
        "unused should still be reported, findings: {names:?}"
    );
}
