package com.ex;

import java.util.List;
import java.util.function.Supplier;

public class App {
    private static void staticRefTarget(String s) { System.out.println(s); }
    private void thisRefTarget(String s) { System.out.println(s); }
    private static int compareTarget(String a, String b) { return a.compareTo(b); }
    private void threadTarget() { }
    private static String supplierTarget() { return "x"; }
    private void reflectTarget() { }
    private void lambdaTarget(String s) { }
    private void deadControl() { }
    private void calledControl() { }

    public void run(List<String> xs) throws Exception {
        xs.forEach(App::staticRefTarget);
        xs.forEach(this::thisRefTarget);
        xs.sort(App::compareTarget);
        new Thread(this::threadTarget).start();
        Supplier<String> s = App::supplierTarget;
        getClass().getDeclaredMethod("reflectTarget").invoke(this);
        xs.forEach(x -> lambdaTarget(x));
        calledControl();
    }

    public static void main(String[] args) throws Exception {
        new App().run(List.of("a"));
    }
}
