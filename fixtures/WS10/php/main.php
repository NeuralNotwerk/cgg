<?php
require_once 'Foo.php';
function run(Foo $f, ?Foo $n) {
    $f->m();
    ($f)->m();
    (new Foo())->m();
    $n?->m();
    $f?->m();
}
