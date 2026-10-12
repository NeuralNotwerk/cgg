<?php
require_once 'Processor.php';
class Zeta implements Processor {
    public function process() { echo "Zeta"; }
    public function unusedOp() { echo "u"; }
}
