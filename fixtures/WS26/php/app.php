<?php
function fmt($x) { return $x; }
function asked() {}          // near-miss: only is_callable
function orphan() {}         // control: dead
function in_string() {}      // near-miss: a string outside a callable position
function first_class() {}

class Helper {
    public static function keep($x) { return true; }
}

class Sorter {
    public function cmp($a, $b) { return $a <=> $b; }
    public static function load($c) {}
    public function unused() {}  // control: dead
    public function run($xs) {
        array_map('fmt', $xs);
        usort($xs, [$this, 'cmp']);
        spl_autoload_register([self::class, 'load']);
        array_filter($xs, [Helper::class, 'keep']);
        is_callable('asked');
        echo 'in_string';
        $f = first_class(...);
        return $f;
    }
}

(new Sorter())->run([2, 1]);
