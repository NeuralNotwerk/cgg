<?php
require_once 'Alpha.php';
require_once 'Beta.php';
require_once 'Gamma.php';
require_once 'Delta.php';
require_once 'Epsilon.php';
require_once 'Zeta.php';

class Holder {
    private Processor $field;
    public function __construct(Processor $p) { $this->field = $p; }
    public function runField() { $this->field->process(); }
}

function main() {
    (new Holder(new Alpha()))->runField();
    (new Holder(new Beta()))->runField();
}

main();
