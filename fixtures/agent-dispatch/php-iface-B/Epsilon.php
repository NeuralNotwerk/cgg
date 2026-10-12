<?php
require_once 'Processor.php';
class Epsilon implements Processor {
    public function process() { echo "Epsilon"; }
    public function unusedOp() { echo "u"; }
}
