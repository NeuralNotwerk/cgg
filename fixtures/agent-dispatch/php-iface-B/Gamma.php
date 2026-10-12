<?php
require_once 'Processor.php';
class Gamma implements Processor {
    public function process() { echo "Gamma"; }
    public function unusedOp() { echo "u"; }
}
