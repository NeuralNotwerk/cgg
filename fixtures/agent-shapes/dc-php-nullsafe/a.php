<?php
class A {
    private ?A $next = null;
    private function hop() { return 1; }
    public function walk() { return $this->next?->hop(); }
}
