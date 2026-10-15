<?php
namespace Tests\Unit;
use PHPUnit\Framework\Attributes\Test;
use Tests\TestCase;
class ATest extends TestCase {
    #[Test]
    public function it_adds() { $this->helper(); }
    private function helper() {}
}
