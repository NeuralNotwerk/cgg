<?php
namespace Ex;

function map_target($x) { return $x + 1; }
function dead_control() { return 0; }
function called_control() { return 1; }

class App {
    private function methodCallable($x) { return $x; }
    private static function staticCallable($x) { return $x; }
    private function callUserFuncTarget() { return 1; }
    private function privDead() { return 0; }
    private function privCalled() { return 2; }
    private function firstClass($x) { return $x; }

    public function run(array $xs) {
        $a = array_map('Ex\map_target', $xs);
        $b = array_map([$this, 'methodCallable'], $xs);
        $c = array_map([self::class, 'staticCallable'], $xs);
        call_user_func([$this, 'callUserFuncTarget']);
        $d = array_map($this->firstClass(...), $xs);
        $this->privCalled();
        called_control();
        return [$a, $b, $c, $d];
    }
}
