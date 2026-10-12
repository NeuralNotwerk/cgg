<?php
use PHPUnit\Framework\TestCase;
use PHPUnit\Framework\Attributes\DataProvider;

class MathTest extends TestCase {
    public static function additionProvider(): array { return [[1, 2]]; }
    private static function privProvider(): array { return [[1]]; }
    public static function docProvider(): array { return [[1]]; }

    #[DataProvider('additionProvider')]
    public function testAdd($a, $b) { $this->assertTrue(true); }

    #[DataProvider('privProvider')]
    public function testPriv($a) { }

    /** @dataProvider docProvider */
    public function testDoc($a) { }

    private function helperDead() { }
}
