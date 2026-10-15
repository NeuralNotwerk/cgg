<?php
use PHPUnit\Framework\TestCase;
class FooTest extends TestCase {
  protected function setUp(): void {}
  public function testA() { $this->helper(); }
  private function helper() {}
  private function unusedP() {}
}
