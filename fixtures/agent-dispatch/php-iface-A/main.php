<?php
require_once 'Alpha.php';
require_once 'Beta.php';

function run(Processor $p) {
    $p->process();
}

function main() {
    run(new Alpha());
    run(new Beta());
}

main();
