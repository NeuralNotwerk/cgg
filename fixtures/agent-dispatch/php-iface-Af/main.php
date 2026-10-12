<?php
require_once 'Alpha.php';
require_once 'Beta.php';

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
