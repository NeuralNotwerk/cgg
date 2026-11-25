//! Framework rules for the scripting languages (Ruby, PHP, Perl, Lua,
//! Bash, PowerShell, R, Julia): one fixture per rule row, asserting the
//! entry (or root mark) lands on the callable the host calls and not on
//! its same-named neighbours.

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
/// each also as `<path>@<qualified name>`; root qualified names with
/// their labels; stderr).
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
        .flat_map(|f| {
            let qn = f["qualified_name"].as_str().unwrap().to_string();
            let root = dir.to_string_lossy();
            let path = f["path"].as_str().unwrap_or_default();
            let path = path
                .strip_prefix(root.as_ref())
                .unwrap_or(path)
                .trim_start_matches(['.', '/']);
            let at = format!("{path}@{qn}");
            [qn, at]
        })
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
// Lua
// ---------------------------------------------------------------------

#[test]
fn love2d_roots_the_love_callbacks_and_not_a_game_objects_update() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "main.lua",
        "local Player = {}\n\
         function Player:update(dt) end\n\
         function Player:draw() end\n\
         function love.load() end\n\
         function love.update(dt) end\n\
         love.draw = function() end\n",
    );
    write(tmp.path(), "conf.lua", "function love.conf(t) end\n");
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("love2d"), "{err}");
    for live in ["love.load", "love.update", "love.conf"] {
        assert!(!dead.contains(live), "{live} is a LÖVE callback: {dead:?}");
    }
    for d in ["Player:update", "Player:draw"] {
        assert!(dead.contains(d), "{d} is no LÖVE callback: {dead:?}");
    }
}

#[test]
fn neovim_setup_is_an_entry_only_in_the_plugin_module() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lua/oil/init.lua",
        "local M = {}\nfunction M.setup(opts)\n  vim.api.nvim_create_autocmd('BufEnter', {})\nend\nreturn M\n",
    );
    write(
        tmp.path(),
        "lua/oil/health.lua",
        "local M = {}\nfunction M.check() end\nreturn M\n",
    );
    write(
        tmp.path(),
        "lua/oil/view.lua",
        "local M = {}\nfunction M.setup() end\nreturn M\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("neovim-plugin"), "{err}");
    assert!(
        names.iter().any(|n| n.ends_with("neovim-plugin::M.setup")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("neovim-plugin::M.check")),
        "{names:?}"
    );
    assert_eq!(names.len(), 2, "view.lua's setup is no entry: {names:?}");
}

#[test]
fn kong_and_apisix_phase_methods_are_entries_only_in_plugin_files() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "kong/plugins/oidc/handler.lua",
        "local kong = require 'kong'\nlocal H = {}\nfunction H:access(conf) end\nreturn H\n",
    );
    write(
        tmp.path(),
        "kong/plugins/oidc/session.lua",
        "local M = {}\nfunction M.configure() end\nreturn M\n",
    );
    write(
        tmp.path(),
        "apisix/plugins/limit-req.lua",
        "local core = require('apisix.core')\nlocal _M = {}\nfunction _M.access(conf, ctx) end\nfunction _M.check_schema(conf) end\nreturn _M\n",
    );
    write(
        tmp.path(),
        "apisix/core/log.lua",
        "local core = require('apisix.core')\nlocal _M = {}\nfunction _M.log() end\nreturn _M\n",
    );
    let (names, _) = entries(tmp.path());
    let has = |s: &str| names.iter().any(|n| n.ends_with(s));
    assert!(has("kong::H:access"), "{names:?}");
    assert!(!has("kong::M.configure"), "{names:?}");
    assert!(has("apisix-plugin::_M.access"), "{names:?}");
    assert!(has("apisix-plugin::_M.check_schema"), "{names:?}");
    assert!(!has("apisix-plugin::_M.log"), "{names:?}");
}

#[test]
fn luanti_definition_callbacks_are_roots() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "mods/chest/init.lua",
        "local chest_def = {}\n\
         function chest_def.on_construct(pos) end\n\
         function chest_def.allow_metadata_inventory_put(pos) return 0 end\n\
         local function unused_helper() end\n\
         minetest.register_node('chest:chest', chest_def)\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("luanti"), "{err}");
    assert!(!dead.contains("chest_def.on_construct"), "{dead:?}");
    assert!(
        !dead.contains("chest_def.allow_metadata_inventory_put"),
        "{dead:?}"
    );
    assert!(dead.contains("unused_helper"), "{dead:?}");
}

#[test]
fn busted_blocks_are_test_entries_of_one_harness_node() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "src/calc.lua",
        "local M = {}\nfunction M.add(a, b) return a + b end\nfunction M.unused() end\nreturn M\n",
    );
    write(
        tmp.path(),
        "spec/calc_spec.lua",
        "local calc = require('calc')\n\
         describe('calc', function()\n\
         \x20 before_each(function() end)\n\
         \x20 it('adds', function()\n    assert.equals(3, calc.add(1, 2))\n  end)\n\
         end)\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("busted (test"), "{err}");
    assert!(
        names.iter().any(|n| n.contains("test::busted")),
        "one harness node: {names:?}"
    );
}

// ---------------------------------------------------------------------
// Bash
// ---------------------------------------------------------------------

#[test]
fn bats_test_blocks_are_entries_and_their_callees_live() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "lib/calc.sh",
        "add() { echo $(( $1 + $2 )); }\nunused() { :; }\n",
    );
    write(
        tmp.path(),
        "test/calc.bats",
        "#!/usr/bin/env bats\n\
         setup() {\n  source lib/calc.sh\n}\n\
         @test \"adds two numbers\" {\n  run add 1 2\n  [ \"$output\" = 3 ]\n}\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("bats (test"), "{err}");
    assert!(
        names.iter().any(|n| n.contains("test::bats")),
        "one harness node: {names:?}"
    );
    let (dead, _, _) = dead_code(tmp.path());
    assert!(!dead.contains("add"), "called from the @test: {dead:?}");
    assert!(dead.contains("unused"), "{dead:?}");
}

#[test]
fn shunit2_cases_are_entries_where_its_asserts_are_called() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "acceptance_tests/basic.sh",
        "#!/bin/bash\n\
         setUp() { rm -f out; }\n\
         testBasicEval() {\n  assertEquals 1 1\n}\n\
         test_other() {\n  assertTrue true\n}\n\
         testing_helper() { :; }\n\
         source ./scripts/shunit2\n",
    );
    write(
        tmp.path(),
        "scripts/release.sh",
        "#!/bin/bash\ntestRelease() { :; }\n",
    );
    let (names, err) = entries(tmp.path());
    assert!(err.contains("shunit2 (test"), "{err}");
    let (dead, _, _) = dead_code(tmp.path());
    for live in ["setUp", "testBasicEval", "test_other"] {
        assert!(!dead.contains(live), "{live}: {dead:?} {names:?}");
    }
    assert!(dead.contains("testing_helper"), "{dead:?}");
    assert!(
        dead.contains("testRelease"),
        "a file without shUnit2's asserts is no suite: {dead:?}"
    );
}

#[test]
fn makepkg_functions_are_roots_only_in_a_pkgbuild() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "linux/PKGBUILD",
        "pkgname=x\npkgver() { echo 1; }\nprepare() { :; }\nbuild() { make; }\npackage_x-headers() { :; }\nhelper() { :; }\n",
    );
    write(
        tmp.path(),
        "linux/x.install",
        "#!/bin/bash\npost_install() { :; }\n",
    );
    write(
        tmp.path(),
        "scripts/ci.sh",
        "#!/bin/bash\nbuild() { make; }\n",
    );
    write(
        tmp.path(),
        "initcpio/hooks/myhook",
        "#!/usr/bin/ash\nrun_hook() { :; }\n",
    );
    let (dead, _, err) = dead_code(tmp.path());
    assert!(err.contains("makepkg"), "{err}");
    for live in [
        "pkgver",
        "prepare",
        "package_x-headers",
        "post_install",
        "run_hook",
    ] {
        assert!(!dead.contains(live), "{live}: {dead:?}");
    }
    assert!(dead.contains("helper"), "{dead:?}");
    // `build` is defined twice; only the PKGBUILD one is a makepkg hook,
    // so one copy is still reported.
    assert!(dead.contains("scripts/ci.sh@build"), "{dead:?}");
    assert!(!dead.contains("linux/PKGBUILD@build"), "{dead:?}");
}
