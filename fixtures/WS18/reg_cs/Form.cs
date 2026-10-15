using System;
public class Form {
  public event EventHandler Click;
  void Init() { Click += OnClick; AppDomain.CurrentDomain.UnhandledException += OnCrash; }
  void OnClick(object s, EventArgs e) {}
  void OnCrash(object s, UnhandledExceptionEventArgs e) {}
  void Dead() {}
  public static void Main() { new Form().Init(); }
}
