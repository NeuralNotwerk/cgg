<?php
require_once 'Alpha.php';
require_once 'Beta.php';
require_once 'Gamma.php';
require_once 'Delta.php';
require_once 'Epsilon.php';
require_once 'Zeta.php';

function run($p) {
    $p->process();
}

function main() {
    run(new Alpha());
    run(new Beta());
    run(new Gamma());
    run(new Delta());
    run(new Epsilon());
    run(new Zeta());
}

main();
