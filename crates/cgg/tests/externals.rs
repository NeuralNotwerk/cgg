//! `--externals`: one row per language plugin.
//!
//! Each row is a small tree with an in-tree module the code calls (which
//! must never be reported), a dependency it imports and calls, and the
//! language's process / native / system forms where it has them. The row
//! names the `(kind, library)` pairs the report must contain and the
//! names it must not.
//!
//! `every_registered_language_has_a_row_or_a_reason` fails when a
//! registered plugin id has no row here and is not in `EXTERNALS_NA`, so
//! "every language" is a gate rather than a promise: a new plugin cannot
//! land without saying how it reaches outside its tree.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

fn report(dir: &Path) -> serde_json::Value {
    let out = Command::cargo_bin("cgg")
        .unwrap()
        .arg(dir)
        .args([
            "--externals=all",
            "--externals-format",
            "json",
            "--no-graph",
            "-q",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "cgg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("externals JSON on stdout")
}

/// Languages with no external-library concept cgg can read, and why.
pub const EXTERNALS_NA: &[(&str, &str)] = &[(
    "graphql",
    "SDL has no import mechanism; a schema names only its own types",
)];

struct Row {
    lang: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// `(kind, library)` pairs that must be reported.
    expect: &'static [(&'static str, &'static str)],
    /// Library names that must not be reported (in-tree modules).
    forbid: &'static [&'static str],
}

const ROWS: &[Row] = &[
    Row {
        lang: "rust",
        files: &[
            (
                "src/lib.rs",
                "mod util;\nuse serde_json::Value;\nuse crate::util::helper;\n\
                 #[link(name = \"z\")]\nextern \"C\" { fn compress(); }\n\
                 pub fn run() {\n    helper();\n    let v: Value = serde_json::from_str(\"1\").unwrap();\n\
                 \x20   std::process::Command::new(\"git\").status();\n    std::fs::read(\"x\");\n    unsafe { compress() };\n}\n",
            ),
            ("src/util.rs", "pub fn helper() {}\n"),
        ],
        expect: &[
            ("third-party", "serde_json"),
            ("process", "git"),
            ("stdlib", "std"),
            ("native", "z"),
        ],
        forbid: &["util", "crate"],
    },
    Row {
        lang: "python",
        files: &[
            ("app/__init__.py", ""),
            ("app/util.py", "def helper():\n    pass\n"),
            (
                "app/main.py",
                "import requests\nimport subprocess\nimport ctypes\nfrom app import util\n\
                 def run():\n    util.helper()\n    requests.get('u')\n    subprocess.run(['git', 'status'])\n    ctypes.CDLL('libz.so.1')\n",
            ),
        ],
        expect: &[
            ("third-party", "requests"),
            ("process", "git"),
            ("native", "libz.so.1"),
        ],
        forbid: &["app", "util"],
    },
    Row {
        lang: "kivy",
        files: &[
            ("app/__init__.py", ""),
            ("app/main.py", "import kivy\ndef go():\n    pass\n"),
            (
                "app/ui.kv",
                "#:import webbrowser webbrowser\n#:import mdc kivymd.uix.button\n<Root>:\n    on_press: webbrowser.open('u')\n",
            ),
        ],
        expect: &[("third-party", "kivymd")],
        forbid: &["app"],
    },
    Row {
        lang: "javascript",
        files: &[
            (
                "src/util.js",
                "function helper() {}\nmodule.exports = { helper };\n",
            ),
            (
                "src/index.js",
                "const axios = require('axios');\nconst { helper } = require('./util');\nconst cp = require('child_process');\n\
                 function run() { helper(); axios.get('u'); cp.execSync('ls -la'); }\nmodule.exports = { run };\n",
            ),
        ],
        expect: &[("third-party", "axios"), ("process", "ls")],
        forbid: &["./util", "util"],
    },
    Row {
        lang: "typescript",
        files: &[
            ("src/util.ts", "export function helper() {}\n"),
            (
                "src/index.ts",
                "import express from 'express';\nimport { helper } from './util';\nexport function run() { helper(); express(); }\n",
            ),
        ],
        expect: &[("third-party", "express")],
        forbid: &["./util", "util"],
    },
    Row {
        lang: "go",
        files: &[
            ("go.mod", "module example.com/app\n"),
            ("util/util.go", "package util\n\nfunc Helper() {}\n"),
            (
                "main.go",
                "package main\n\nimport (\n\t\"os/exec\"\n\n\t\"example.com/app/util\"\n\t\"github.com/google/uuid\"\n)\n\n\
                 func run() {\n\tutil.Helper()\n\tuuid.NewString()\n\texec.Command(\"git\", \"status\")\n}\n",
            ),
        ],
        expect: &[
            ("third-party", "github.com/google/uuid"),
            ("process", "git"),
        ],
        forbid: &["example.com/app", "example.com/app/util"],
    },
    Row {
        lang: "java",
        files: &[
            (
                "com/acme/util/Helper.java",
                "package com.acme.util;\npublic class Helper { public static void help() {} }\n",
            ),
            (
                "com/acme/Main.java",
                "package com.acme;\nimport com.google.common.collect.ImmutableList;\nimport com.acme.util.Helper;\n\
                 class Main { void run() throws Exception { Helper.help(); ImmutableList.of(); Runtime.getRuntime().exec(\"git status\"); System.loadLibrary(\"native\"); } }\n",
            ),
        ],
        expect: &[
            ("third-party", "com.google.common"),
            ("process", "git"),
            ("native", "native"),
        ],
        forbid: &["com.acme"],
    },
    Row {
        lang: "kotlin",
        files: &[
            ("a/Util.kt", "package a\nfun helper() {}\n"),
            (
                "a/Main.kt",
                "package a\nimport okhttp3.OkHttpClient\nfun run() { helper(); OkHttpClient().newBuilder() }\n",
            ),
        ],
        expect: &[("third-party", "okhttp3")],
        forbid: &["a"],
    },
    Row {
        lang: "c",
        files: &[
            ("util.h", "void helper(void);\n"),
            ("util.c", "#include \"util.h\"\nvoid helper(void) {}\n"),
            (
                "main.c",
                "#include <curl/curl.h>\n#include <unistd.h>\n#include <dlfcn.h>\n#include <stdlib.h>\n#include \"util.h\"\n\
                 void run(void) { helper(); curl_easy_init(); fork(); system(\"ls -la\"); dlopen(\"libm.so.6\", 0); }\n",
            ),
        ],
        expect: &[
            ("third-party", "curl"),
            ("system", "posix"),
            ("process", "ls"),
            ("native", "libm.so.6"),
        ],
        forbid: &["util"],
    },
    Row {
        lang: "cpp",
        files: &[
            ("util.hpp", "namespace app { void helper(); }\n"),
            (
                "util.cpp",
                "#include \"util.hpp\"\nnamespace app { void helper() {} }\n",
            ),
            (
                "main.cpp",
                "#include <boost/algorithm/string.hpp>\n#include \"util.hpp\"\nvoid run() { app::helper(); boost::split(); }\n",
            ),
        ],
        expect: &[("third-party", "boost")],
        forbid: &["app", "util"],
    },
    Row {
        lang: "csharp",
        files: &[
            (
                "Util.cs",
                "namespace App { public static class Util { public static void Helper() {} } }\n",
            ),
            (
                "Main.cs",
                "using Newtonsoft.Json;\nusing System.Diagnostics;\nnamespace App {\n class P { void Run() { Util.Helper(); JsonConvert.SerializeObject(1); Process.Start(\"git\"); } }\n}\n",
            ),
        ],
        expect: &[("third-party", "Newtonsoft.Json"), ("process", "git")],
        forbid: &["App"],
    },
    Row {
        lang: "bash",
        files: &[
            ("lib.sh", "helper() { :; }\n"),
            (
                "run.sh",
                "#!/bin/bash\nsource ./lib.sh\nrun() {\n  helper\n  git status\n  jq . f.json\n}\ncurl -s https://x\n",
            ),
        ],
        expect: &[("process", "git"), ("process", "jq"), ("process", "curl")],
        forbid: &["helper"],
    },
    Row {
        lang: "ruby",
        files: &[
            ("util.rb", "module Util\n  def self.helper; end\nend\n"),
            (
                "main.rb",
                "require 'nokogiri'\nrequire_relative 'util'\ndef run\n  Util.helper\n  Nokogiri::HTML('x')\n  system('git status')\nend\n",
            ),
        ],
        expect: &[("third-party", "nokogiri"), ("process", "git")],
        forbid: &["util", "Util"],
    },
    Row {
        lang: "swift",
        files: &[
            ("Util.swift", "func helper() {}\n"),
            (
                "Main.swift",
                "import Alamofire\nfunc run() { helper(); AF.request(\"u\") }\n",
            ),
        ],
        expect: &[("third-party", "Alamofire")],
        forbid: &["Util"],
    },
    Row {
        lang: "lua",
        files: &[
            (
                "util.lua",
                "local M = {}\nfunction M.helper() end\nreturn M\n",
            ),
            (
                "main.lua",
                "local cjson = require(\"cjson\")\nlocal util = require(\"util\")\nfunction run()\n  util.helper()\n  cjson.encode({})\n  os.execute(\"ls -la\")\nend\n",
            ),
        ],
        expect: &[("third-party", "cjson"), ("process", "ls")],
        forbid: &["util"],
    },
    Row {
        lang: "php",
        files: &[
            (
                "src/Util.php",
                "<?php\nnamespace App;\nclass Util { public static function helper() {} }\n",
            ),
            (
                "src/main.php",
                "<?php\nnamespace App;\nuse GuzzleHttp\\Client;\nfunction run() { Util::helper(); $c = new Client(); exec(\"ls -la\"); }\n",
            ),
        ],
        expect: &[("third-party", "GuzzleHttp\\Client"), ("process", "ls")],
        forbid: &["App"],
    },
    Row {
        lang: "dart",
        files: &[
            ("pubspec.yaml", "name: app\n"),
            ("lib/util.dart", "void helper() {}\n"),
            (
                "lib/main.dart",
                "import 'package:http/http.dart' as http;\nimport 'util.dart';\nimport 'dart:io';\nvoid run() { helper(); http.get(Uri()); Process.run('git', []); }\n",
            ),
        ],
        expect: &[("third-party", "http"), ("process", "git")],
        forbid: &["util.dart", "app"],
    },
    Row {
        lang: "scala",
        files: &[
            (
                "a/Util.scala",
                "package a\nobject Util { def helper(): Unit = () }\n",
            ),
            (
                "a/Main.scala",
                "package a\nimport cats.effect.IO\nobject Main { def run(): Unit = { Util.helper(); IO.println(\"x\") } }\n",
            ),
        ],
        expect: &[("third-party", "cats.effect")],
        forbid: &["a"],
    },
    Row {
        lang: "hcl",
        files: &[
            (
                "main.tf",
                "module \"vpc\" {\n  source = \"terraform-aws-modules/vpc/aws\"\n}\nmodule \"local\" {\n  source = \"./modules/local\"\n}\nprovider \"aws\" {\n  region = \"us-east-1\"\n}\n",
            ),
            ("modules/local/main.tf", "variable \"x\" {}\n"),
        ],
        expect: &[
            ("third-party", "terraform-aws-modules/vpc/aws"),
            ("third-party", "aws"),
        ],
        forbid: &["./modules/local"],
    },
    Row {
        lang: "zig",
        files: &[
            ("src/util.zig", "pub fn helper() void {}\n"),
            (
                "src/main.zig",
                "const std = @import(\"std\");\nconst zap = @import(\"zap\");\nconst util = @import(\"util.zig\");\npub fn run() void {\n    util.helper();\n    zap.start();\n}\n",
            ),
        ],
        expect: &[("third-party", "zap")],
        forbid: &["util.zig"],
    },
    Row {
        lang: "objc",
        files: &[
            ("Util.h", "@interface Util\n+ (void)helper;\n@end\n"),
            (
                "Util.m",
                "#import \"Util.h\"\n@implementation Util\n+ (void)helper {}\n@end\n",
            ),
            (
                "Main.m",
                "#import <AFNetworking/AFNetworking.h>\n#import \"Util.h\"\n@implementation Main\n- (void)run { [Util helper]; [AFHTTPSessionManager manager]; }\n@end\n",
            ),
        ],
        expect: &[("third-party", "AFNetworking")],
        forbid: &["Util"],
    },
    Row {
        lang: "r",
        files: &[
            ("R/util.R", "helper <- function() {}\n"),
            (
                "R/main.R",
                "library(dplyr)\nrun <- function(df) {\n  helper()\n  dplyr::filter(df)\n  system(\"ls -la\")\n}\n",
            ),
        ],
        expect: &[("third-party", "dplyr"), ("process", "ls")],
        forbid: &["util"],
    },
    Row {
        lang: "groovy",
        files: &[
            (
                "a/Util.groovy",
                "package a\nclass Util { static void helper() {} }\n",
            ),
            (
                "a/Main.groovy",
                "package a\nimport org.apache.commons.lang3.StringUtils\nclass Main { def run() { Util.helper(); StringUtils.isBlank('x') } }\n",
            ),
        ],
        expect: &[("third-party", "org.apache.commons")],
        forbid: &["a"],
    },
    Row {
        lang: "julia",
        files: &[
            ("src/util.jl", "function helper() end\n"),
            (
                "src/main.jl",
                "using HTTP\ninclude(\"util.jl\")\nfunction run()\n  helper()\n  HTTP.get(\"u\")\nend\n",
            ),
        ],
        expect: &[("third-party", "HTTP")],
        forbid: &["util"],
    },
    Row {
        lang: "perl",
        files: &[
            (
                "lib/App/Util.pm",
                "package App::Util;\nsub helper { }\n1;\n",
            ),
            (
                "lib/App/Main.pm",
                "package App::Main;\nuse LWP::UserAgent;\nuse App::Util;\nsub run { App::Util::helper(); my $ua = LWP::UserAgent->new; system(\"ls -la\"); }\n1;\n",
            ),
        ],
        expect: &[("third-party", "LWP"), ("process", "ls")],
        forbid: &["App"],
    },
    Row {
        lang: "elixir",
        files: &[
            (
                "lib/util.ex",
                "defmodule App.Util do\n  def helper, do: :ok\nend\n",
            ),
            (
                "lib/app.ex",
                "defmodule App do\n  alias App.Util\n  def run do\n    Util.helper()\n    Jason.encode!(%{})\n    System.cmd(\"git\", [\"status\"])\n  end\nend\n",
            ),
        ],
        expect: &[("third-party", "Jason"), ("process", "git")],
        forbid: &["App"],
    },
    Row {
        lang: "erlang",
        files: &[
            (
                "src/util.erl",
                "-module(util).\n-export([helper/0]).\nhelper() -> ok.\n",
            ),
            (
                "src/app.erl",
                "-module(app).\n-export([run/0]).\nrun() -> util:helper(), jsx:encode(#{}), os:cmd(\"ls -la\").\n",
            ),
        ],
        expect: &[("third-party", "jsx"), ("process", "ls")],
        forbid: &["util"],
    },
    Row {
        lang: "fortran",
        files: &[
            (
                "util.f90",
                "module util\ncontains\n  subroutine helper()\n  end subroutine\nend module\n",
            ),
            (
                "main.f90",
                "program p\n  use util\n  use mpi\n  integer :: ierr\n  call helper()\n  call MPI_Init(ierr)\nend program\n",
            ),
        ],
        expect: &[("third-party", "mpi")],
        forbid: &["util"],
    },
    Row {
        lang: "clojure",
        files: &[
            ("src/app/util.clj", "(ns app.util)\n(defn helper [] 1)\n"),
            (
                "src/app/core.clj",
                "(ns app.core (:require [clj-http.client :as http] [app.util :as u]))\n(defn run [] (u/helper) (http/get \"u\"))\n",
            ),
        ],
        expect: &[("third-party", "clj-http")],
        forbid: &["app"],
    },
    Row {
        lang: "haskell",
        files: &[
            (
                "src/Util.hs",
                "module Util where\nhelper :: Int\nhelper = 1\n",
            ),
            (
                "src/Main.hs",
                "module Main where\nimport Util\nimport qualified Data.Aeson as A\nimport qualified Data.Text as T\nmain = print (A.encode helper)\nf x = T.pack x\n",
            ),
        ],
        expect: &[("third-party", "Data.Aeson"), ("stdlib", "Data.Text")],
        forbid: &["Util"],
    },
    Row {
        lang: "ocaml",
        files: &[
            ("util.ml", "let helper () = ()\n"),
            ("main.ml", "let run () = Util.helper (); Lwt.return ()\n"),
        ],
        expect: &[("third-party", "Lwt")],
        forbid: &["Util"],
    },
    Row {
        lang: "powershell",
        files: &[
            ("lib.ps1", "function Invoke-Helper { }\n"),
            (
                "main.ps1",
                ". ./lib.ps1\nfunction Run { Invoke-Helper; git status }\n",
            ),
        ],
        expect: &[("process", "git")],
        forbid: &["Invoke-Helper"],
    },
    Row {
        lang: "solidity",
        files: &[(
            "contracts/A.sol",
            "pragma solidity ^0.8.0;\nimport \"@openzeppelin/contracts/token/ERC20/IERC20.sol\";\ncontract A { function run(address t) public { IERC20(t).transfer(msg.sender, 1); } }\n",
        )],
        expect: &[("third-party", "@openzeppelin/contracts")],
        forbid: &["A"],
    },
    Row {
        lang: "fsharp",
        files: &[
            ("Util.fs", "module Util\nlet helper () = ()\n"),
            (
                "App.fs",
                "module App\nopen Newtonsoft.Json\nlet run () =\n    Util.helper ()\n    JsonConvert.SerializeObject(1)\n",
            ),
        ],
        expect: &[("third-party", "Newtonsoft.Json")],
        forbid: &["Util"],
    },
    Row {
        lang: "starlark",
        files: &[
            ("tools/defs.bzl", "def helper(name):\n    pass\n"),
            (
                "BUILD",
                "load(\"@rules_cc//cc:defs.bzl\", \"cc_library\")\nload(\"//tools:defs.bzl\", \"helper\")\nhelper(name = \"a\")\ncc_library(name = \"x\")\n",
            ),
        ],
        expect: &[("third-party", "@rules_cc")],
        forbid: &["//tools:defs.bzl"],
    },
    Row {
        lang: "cmake",
        files: &[(
            "CMakeLists.txt",
            "cmake_minimum_required(VERSION 3.20)\nfind_package(Boost REQUIRED)\nfunction(run)\n  execute_process(COMMAND git status)\nendfunction()\n",
        )],
        expect: &[("third-party", "Boost"), ("process", "git")],
        forbid: &[],
    },
    Row {
        lang: "nix",
        files: &[(
            "default.nix",
            "{ pkgs ? import <nixpkgs> {} }:\npkgs.stdenv.mkDerivation {\n  src = pkgs.fetchFromGitHub { owner = \"a\"; repo = \"b\"; rev = \"1\"; };\n}\n",
        )],
        expect: &[("third-party", "nixpkgs"), ("third-party", "github:a/b")],
        forbid: &[],
    },
    Row {
        lang: "verilog",
        files: &[
            (
                "rtl/adder.v",
                "module adder(input a, output b); assign b = a; endmodule\n",
            ),
            (
                "rtl/top.v",
                "`include \"uvm_macros.svh\"\nmodule top(input a, output b);\n  adder u0(.a(a), .b(b));\n  IBUF u1(.I(a), .O(b));\nendmodule\n",
            ),
        ],
        expect: &[("third-party", "IBUF"), ("third-party", "uvm_macros.svh")],
        forbid: &["adder"],
    },
    Row {
        lang: "vhdl",
        files: &[(
            "e.vhd",
            "library ieee;\nuse ieee.std_logic_1164.all;\nlibrary unisim;\nuse unisim.vcomponents.all;\nentity e is end e;\narchitecture rtl of e is begin end rtl;\n",
        )],
        expect: &[("third-party", "unisim"), ("stdlib", "ieee")],
        forbid: &["work", "e"],
    },
    Row {
        lang: "asm",
        files: &[(
            "start.s",
            ".intel_syntax noprefix\n.global _start\n.extern printf\n_start:\n  call printf\n  mov rax, 60\n  syscall\n",
        )],
        expect: &[("system", "syscall"), ("stdlib", "libc")],
        forbid: &["_start"],
    },
    Row {
        lang: "smithy",
        files: &[(
            "model/main.smithy",
            "$version: \"2\"\nnamespace a.b\nuse aws.protocols#restJson1\n@restJson1\nservice S { version: \"1\" }\n",
        )],
        expect: &[("third-party", "aws.protocols")],
        forbid: &["a.b"],
    },
    Row {
        lang: "proto",
        files: &[
            (
                "a/local.proto",
                "syntax = \"proto3\";\npackage a;\nmessage L {}\n",
            ),
            (
                "a/m.proto",
                "syntax = \"proto3\";\npackage a;\nimport \"google/protobuf/timestamp.proto\";\nimport \"vendor/ext.proto\";\nimport \"a/local.proto\";\nmessage M { google.protobuf.Timestamp t = 1; }\n",
            ),
        ],
        expect: &[
            ("stdlib", "google/protobuf/timestamp.proto"),
            ("third-party", "vendor/ext.proto"),
        ],
        forbid: &["a/local.proto"],
    },
    Row {
        lang: "openapi",
        files: &[(
            "api.yaml",
            "openapi: 3.0.0\ninfo: {title: t, version: '1'}\npaths: {}\ncomponents:\n  schemas:\n    Pet:\n      $ref: 'https://example.com/schemas/pet.yaml#/Pet'\n    Local:\n      $ref: '#/components/schemas/Pet'\n",
        )],
        expect: &[("third-party", "https://example.com/schemas/pet.yaml")],
        forbid: &["#/components/schemas/Pet"],
    },
    Row {
        lang: "asyncapi",
        files: &[(
            "api.yaml",
            "asyncapi: 2.6.0\ninfo: {title: t, version: '1'}\nchannels: {}\ncomponents:\n  messages:\n    M:\n      payload:\n        $ref: 'common/schemas.yaml#/Thing'\n",
        )],
        expect: &[("third-party", "common/schemas.yaml")],
        forbid: &[],
    },
    Row {
        lang: "lean",
        files: &[
            ("App/Util.lean", "def helper := 1\n"),
            (
                "App/Main.lean",
                "import Mathlib.Data.Nat.Basic\nimport App.Util\ndef run := helper\n",
            ),
        ],
        expect: &[("third-party", "Mathlib")],
        forbid: &["App"],
    },
];

fn libs(doc: &serde_json::Value) -> Vec<(String, String)> {
    doc["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["kind"].as_str().unwrap().to_string(),
                l["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn check(row: &Row) -> Result<(), String> {
    let tmp = TempDir::new().unwrap();
    for (name, body) in row.files {
        write(tmp.path(), name, body);
    }
    let doc = report(tmp.path());
    let got = libs(&doc);
    let mut errs = Vec::new();
    for (kind, lib) in row.expect {
        if !got.iter().any(|(k, l)| k == kind && l == lib) {
            errs.push(format!("missing ({kind}, {lib})"));
        }
    }
    for lib in row.forbid {
        if got.iter().any(|(_, l)| l == lib) {
            errs.push(format!("in-tree `{lib}` reported as external"));
        }
    }
    // Accounting: every site counted once.
    let t = &doc["totals"];
    let by_kind: u64 = t["by_kind"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    if by_kind != t["sites"].as_u64().unwrap() {
        errs.push(format!(
            "totals.by_kind {by_kind} != totals.sites {}",
            t["sites"]
        ));
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: {} — got {:?}", row.lang, errs.join("; "), got))
    }
}

#[test]
fn every_language_reports_its_dependencies_and_never_its_own_modules() {
    let failures: Vec<String> = ROWS.iter().filter_map(|r| check(r).err()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} languages failed:\n{}",
        failures.len(),
        ROWS.len(),
        failures.join("\n")
    );
}

#[test]
fn every_registered_language_has_a_row_or_a_reason() {
    let reg = cgg_lang::PluginRegistry::with_v1_plugins();
    let missing: Vec<&str> = reg
        .all()
        .iter()
        .map(|p| p.id())
        .filter(|id| {
            !ROWS.iter().any(|r| r.lang == *id)
                && !EXTERNALS_NA.iter().any(|(l, _)| l == id)
        })
        .collect();
    assert!(missing.is_empty(), "no externals row for: {missing:?}");
}

#[test]
fn default_run_is_unchanged_and_in_tree_edges_survive_the_flag() {
    // The same tree, with and without the flag: every in-tree edge of the
    // default graph is still in the externals graph or was pruned for not
    // reaching an external call — never rewritten.
    let tmp = TempDir::new().unwrap();
    for (name, body) in ROWS[1].files {
        write(tmp.path(), name, body);
    }
    let run = |extra: &[&str]| -> serde_json::Value {
        let out = Command::cargo_bin("cgg")
            .unwrap()
            .arg(tmp.path())
            .args(["-t", "json", "-q"])
            .args(extra)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let plain = run(&[]);
    // Discarded; `/dev/null` does not exist on Windows.
    let sink = TempDir::new().unwrap();
    let report = sink.path().join("externals.txt");
    let ext = run(&[
        "--externals=all",
        "--externals-report",
        report.to_str().unwrap(),
    ]);
    let key = |e: &serde_json::Value| {
        (
            e["src"].as_str().unwrap().to_string(),
            e["dst"].as_str().unwrap().to_string(),
            e["site_byte"].as_u64().unwrap(),
            e["confidence"].as_str().unwrap().to_string(),
        )
    };
    let ext_edges: std::collections::HashSet<_> =
        ext["edges"].as_array().unwrap().iter().map(key).collect();
    let ext_nodes = ext["callables"].as_object().unwrap();
    for e in plain["edges"].as_array().unwrap() {
        let k = key(e);
        let both_kept = ext_nodes.contains_key(&k.0) && ext_nodes.contains_key(&k.1);
        assert!(
            !both_kept || ext_edges.contains(&k),
            "edge changed under --externals: {k:?}"
        );
    }
}
