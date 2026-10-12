package com.ex;

import java.util.stream.Stream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.MethodSource;
import org.junit.jupiter.params.provider.Arguments;

class AppTest {
    private static Stream<Arguments> provider() { return Stream.of(Arguments.of(1)); }
    static Stream<Arguments> pkgProvider() { return Stream.of(Arguments.of(2)); }
    private static void testDeadHelper() { }

    @ParameterizedTest
    @MethodSource("provider")
    void usesProvider(int x) { }

    @ParameterizedTest
    @MethodSource("pkgProvider")
    void usesPkgProvider(int x) { }

    @Test
    void plain() { }
}
