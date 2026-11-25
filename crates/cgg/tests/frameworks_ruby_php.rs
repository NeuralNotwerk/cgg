//! Framework rules for Ruby and PHP: one fixture per rule row, asserting
//! the entry (or root mark) lands on the callable the host calls and not
//! on its same-named neighbours.

use std::collections::BTreeSet;
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

/// Run `--dead-code` over `dir` and return (qualified names reported,
/// root qualified names with their labels, stderr).
fn dead_code(dir: &Path) -> (BTreeSet<String>, Vec<(String, String)>, String) {
    let out = cgg()
        .args([
            "--dead-code",
            "--no-graph",
            "--dead-code-format",
            "json",
            "--dead-code-confidence",
            "low",
            "--jobs",
            "1",
        ])
        .arg(dir)
        .assert()
        .success();
    let output = out.get_output();
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json report");
    let findings = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["qualified_name"].as_str().unwrap().to_string())
        .collect();
    let roots = doc["roots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["qualified_name"].as_str().unwrap_or_default().to_string(),
                r.to_string(),
            )
        })
        .collect();
    (
        findings,
        roots,
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// The framework entry nodes of a run: `<framework-entry>::…` names.
fn entries(dir: &Path) -> (BTreeSet<String>, String) {
    let out = cgg()
        .args(["-t", "json", "--jobs", "1"])
        .arg(dir)
        .assert()
        .success();
    let output = out.get_output();
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json graph");
    let callables = &doc["callables"];
    let list: Vec<&serde_json::Value> = match callables {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(o) => o.values().collect(),
        _ => Vec::new(),
    };
    let names = list
        .iter()
        .filter_map(|c| c["qualified_name"].as_str())
        .filter(|q| q.starts_with("<framework-entry>"))
        .map(str::to_string)
        .collect();
    (names, String::from_utf8_lossy(&output.stderr).to_string())
}

// ---------------------------------------------------------------------
// Ruby
// ---------------------------------------------------------------------

#[test]
fn ar_migration_change_up_down_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "db/migrate/20240101000000_add_x.rb",
        "class AddX < ActiveRecord::Migration[7.1]\n  def change; add_column :a, :b, :c; end\n  def up; end\n  def down; end\n  def stray; end\nend\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("ar-migration"), "{err}");
    for live in ["AddX::change", "AddX::up", "AddX::down"] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("AddX::stray"), "{dead:?}");
}

#[test]
fn pundit_predicates_and_scope_resolve_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/policies/application_policy.rb",
        "class ApplicationPolicy\n  def initialize(u, r); end\nend\n",
    );
    write(
        tmp.path(),
        "app/policies/post_policy.rb",
        "class PostPolicy < ApplicationPolicy\n  def update?; true; end\n  def helper_text; end\n  class Scope < ApplicationPolicy::Scope\n    def resolve; end\n  end\nend\n",
    );
    write(
        tmp.path(),
        "app/policies/app_policy.rb",
        "class AppPolicy < ActionPolicy::Base\n  def show?; true; end\nend\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("pundit"), "{err}");
    for live in [
        "PostPolicy::update?",
        "PostPolicy::Scope::resolve",
        "AppPolicy::show?",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("PostPolicy::helper_text"), "{dead:?}");
}

#[test]
fn graphql_ruby_type_members_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/graphql/mutations/create_post.rb",
        "module Mutations\n  class CreatePost < GraphQL::Schema::Mutation\n    def resolve(title:); end\n  end\nend\n",
    );
    write(
        tmp.path(),
        "app/models/post.rb",
        "class Post\n  def resolve; end\nend\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("graphql-ruby"), "{err}");
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("graphql-ruby::Mutations::CreatePost::resolve")),
        "{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.contains("Post::resolve") && !n.contains("CreatePost")),
        "{names:?}"
    );
}

#[test]
fn thor_commands_are_entries_and_active_record_models_are_not() {
    // `ActiveRecord::Base` must not walk into the app's own `CLI::Base`.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/mastodon/cli/base.rb",
        "require 'thor'\nmodule Mastodon\n  module CLI\n    class Base < Thor\n      def self.exit_on_failure?; true; end\n    end\n    class Accounts < Base\n      desc 'rotate', 'x'\n      def rotate; end\n    end\n  end\nend\n",
    );
    write(
        tmp.path(),
        "app/models/account.rb",
        "class ApplicationRecord < ActiveRecord::Base\n  def touch_all; end\nend\nclass Account < ApplicationRecord\n  def suspend!; end\nend\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("thor"), "{err}");
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("cli::thor::Mastodon::CLI::Accounts::rotate")),
        "{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|n| n.contains("Account::suspend!") || n.contains("touch_all")),
        "a model is no Thor command: {names:?}"
    );
}

#[test]
fn validators_components_and_serializers_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/validators/email_validator.rb",
        "class EmailValidator < ActiveModel::EachValidator\n  def validate_each(r, a, v); end\n  def unused_x; end\nend\n",
    );
    write(
        tmp.path(),
        "app/components/button_component.rb",
        "class ButtonComponent < ViewComponent::Base\n  def label; end\nend\nclass ApplicationView < Phlex::HTML\nend\nclass HomeView < ApplicationView\n  def view_template; end\nend\n",
    );
    write(
        tmp.path(),
        "app/serializers/post_serializer.rb",
        "class PostSerializer < ActiveModel::Serializer\n  attributes :url\n  def url; end\nend\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("active-model-validators"), "{err}");
    assert!(err.contains("view-component"), "{err}");
    assert!(err.contains("active-model-serializers"), "{err}");
    for live in [
        "EmailValidator::validate_each",
        "ButtonComponent::label",
        "PostSerializer::url",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("EmailValidator::unused_x"), "{dead:?}");
}

// ---------------------------------------------------------------------
// PHP
// ---------------------------------------------------------------------

#[test]
fn livewire_public_actions_are_entries_and_private_helpers_and_blade_components_are_not()
{
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/Livewire/Counter.php",
        "<?php\nnamespace App\\Livewire;\nuse Livewire\\Component;\nuse Livewire\\Attributes\\On;\n\
         class Counter extends Component {\n\
         \x20 public function mount() {}\n\
         \x20 public function increment() { $this->bump(); }\n\
         \x20 #[On('reset')]\n  protected function onReset() {}\n\
         \x20 private function bump() {}\n\
         \x20 private function unusedHelper() {}\n\
         \x20 public static function make() {}\n}\n",
    );
    write(
        tmp.path(),
        "app/View/Components/Alert.php",
        "<?php\nnamespace App\\View\\Components;\nuse Illuminate\\View\\Component;\n\
         class Alert extends Component {\n  public function shout() {}\n}\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("livewire"), "{err}");
    let has = |s: &str| names.iter().any(|n| n.ends_with(s));
    assert!(has("livewire::Counter::increment"), "{names:?}");
    assert!(has("livewire::Counter::mount"), "{names:?}");
    assert!(has("livewire::On(\"reset\")"), "{names:?}");
    assert!(!has("livewire::Counter::unusedHelper"), "{names:?}");
    assert!(!has("livewire::Counter::make"), "{names:?}");
    assert!(
        !has("livewire::Alert::shout"),
        "a Blade component is no Livewire one: {names:?}"
    );
}

#[test]
fn filament_resource_hooks_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/Filament/Resources/UserResource.php",
        "<?php\nnamespace App\\Filament\\Resources;\nuse Filament\\Resources\\Resource;\n\
         class UserResource extends Resource {\n\
         \x20 public static function form($f) {}\n\
         \x20 public static function getPages(): array { return []; }\n\
         \x20 public static function canCreate(): bool { return true; }\n\
         \x20 private static function unusedThing() {}\n}\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("filament"), "{err}");
    for live in [
        "UserResource::form",
        "UserResource::getPages",
        "UserResource::canCreate",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("UserResource::unusedThing"), "{dead:?}");
}

#[test]
fn nextcloud_controllers_and_lifecycle_classes() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/Controller/PageController.php",
        "<?php\nnamespace OCA\\Cospend\\Controller;\nuse OCP\\AppFramework\\Controller;\n\
         class PageController extends Controller {\n\
         \x20 #[NoAdminRequired]\n  public function index() {}\n\
         \x20 public function adminOnly() {}\n\
         \x20 private function helper() {}\n}\n",
    );
    write(
        tmp.path(),
        "lib/Cron/Cleanup.php",
        "<?php\nnamespace OCA\\Cospend\\Cron;\nuse OCP\\BackgroundJob\\TimedJob;\n\
         class Cleanup extends TimedJob {\n  protected function run($arg) {}\n  private function unusedCron() {}\n}\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("nextcloud"), "{err}");
    for live in [
        "PageController::index",
        "PageController::adminOnly",
        "Cleanup::run",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    for d in ["PageController::helper", "Cleanup::unusedCron"] {
        assert!(dead.contains(d), "{d}: {dead:?}");
    }
}

#[test]
fn hyperf_mappings_and_commands_are_entries() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "app/Controller/UserController.php",
        "<?php\nnamespace App\\Controller;\nuse Hyperf\\HttpServer\\Annotation\\GetMapping;\n\
         class UserController {\n  #[GetMapping(path: \"/users\")]\n  public function list() {}\n}\n",
    );
    write(
        tmp.path(),
        "app/Command/Sync.php",
        "<?php\nnamespace App\\Command;\nuse Hyperf\\Command\\Command\\HyperfCommand;\n\
         class Sync extends HyperfCommand {\n  public function handle() {}\n}\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("hyperf"), "{err}");
    let has = |s: &str| names.iter().any(|n| n.contains("hyperf") && n.contains(s));
    assert!(has("GetMapping(\"/users\")"), "{names:?}");
    assert!(has("Sync::handle"), "{names:?}");
}

#[test]
fn doctrine_callbacks_match_qualified_attributes_and_api_platform_processors() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/Entity/Part.php",
        "<?php\nnamespace App\\Entity;\nuse Doctrine\\ORM\\Mapping as ORM;\n\
         #[ORM\\Entity]\nclass Part {\n\
         \x20 #[ORM\\PrePersist]\n  public function onPrePersist(): void {}\n\
         \x20 #[\\Doctrine\\ORM\\Mapping\\PreUpdate]\n  public function onUpdate(): void {}\n\
         \x20 public function unusedPart(): void {}\n}\n",
    );
    write(
        tmp.path(),
        "src/State/PartProcessor.php",
        "<?php\nnamespace App\\State;\nuse ApiPlatform\\State\\ProcessorInterface;\n\
         class PartProcessor implements ProcessorInterface {\n  public function process($d, $o, $u = [], $c = []) {}\n}\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("doctrine-lifecycle"), "{err}");
    assert!(err.contains("api-platform"), "{err}");
    for live in [
        "Part::onPrePersist",
        "Part::onUpdate",
        "PartProcessor::process",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("Part::unusedPart"), "{dead:?}");
}

#[test]
fn pest_closures_are_test_entries_and_reach_their_callees() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "tests/Pest.php",
        "<?php\nuses(Tests\\TestCase::class)->in('Feature');\n",
    );
    write(
        tmp.path(),
        "tests/Feature/UserTest.php",
        "<?php\nuse function Pest\\Laravel\\get;\n\
         beforeEach(function () { seedUsers(); });\n\
         it('lists users', function () { expect(listUsers())->toBeArray(); });\n\
         test('counts', fn () => countUsers());\n\
         function seedUsers() {}\n",
    );
    write(
        tmp.path(),
        "src/users.php",
        "<?php\nfunction listUsers() { return []; }\nfunction countUsers() { return 0; }\nfunction unusedUsers() {}\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("pest"), "{err}");
    assert!(
        names.iter().any(|n| n.contains("test::pest")),
        "one pest harness node: {names:?}"
    );
    let (dead, _, _) = dead_code(tmp.path());
    for live in ["listUsers", "countUsers"] {
        assert!(
            !dead.contains(live),
            "{live} is called from a test: {dead:?}"
        );
    }
    assert!(dead.contains("unusedUsers"), "{dead:?}");
}
