<?php
class Foo {
  public function run() { $this->helper(); return self::stat(); }
  private function helper() { return 1; }
  public static function stat() { return 2; }
}
