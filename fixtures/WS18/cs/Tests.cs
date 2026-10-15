using Xunit;
using NUnit.Framework;
using UnityEditor;
using UnityEngine;
using Godot;
using BenchmarkDotNet.Attributes;
using System.Runtime.InteropServices.JavaScript;
public class XT { [Fact] public void A() {} [Theory] public void B(int x) {} void Unused() {} }
public class NT { [Test] public void C() {} [SetUp] public void S() {} }
public class Ed { [MenuItem("Tools/Do")] static void Do() {} [DrawGizmo(GizmoType.Selected)] static void G() {} }
public partial class Ball : Node { [Rpc] void Bounce() {} }
public class Bench { [Benchmark] public void Run() {} [GlobalSetup] public void Setup() {} }
public partial class Ex { [JSExport] internal static string Gen() => ""; }
public class Com { [ComRegisterFunction] static void Reg(System.Type t) {} }
