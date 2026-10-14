<?php
namespace App;
use PHPUnit\Framework\TestCase;
class Kid extends Base { use Helps; public function run() { $this->inherited(); $this->helped(); } }
class KidTest extends TestCase { public function testX() { $this->assertCount(1, []); } }
