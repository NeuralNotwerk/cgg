package demo;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;
import org.junit.jupiter.api.Test;
import java.util.stream.Stream;

class ParamTest {
    @ParameterizedTest
    @MethodSource("words")
    void named(String w) { check(w); }

    @ParameterizedTest
    @MethodSource
    void sameName(String w) { check(w); }

    @ParameterizedTest
    @MethodSource({"alpha", "demo.Shared#beta"})
    void several(String w) { check(w); }

    private static Stream<String> words() { return Stream.of("a"); }
    private static Stream<String> sameName() { return Stream.of("b"); }
    private static Stream<String> alpha() { return Stream.of("c"); }
    // near miss: nothing names it
    private static Stream<String> unusedSource() { return Stream.of("d"); }

    private void check(String w) {}
}

class Shared {
    static Stream<String> beta() { return Stream.of("e"); }
}
