<?php
require_once 'Processor.php';
class Beta implements Processor {
    public function process() { echo "Beta"; }
    public function unusedOp() { echo "u"; }
}
