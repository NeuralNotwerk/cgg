<?php
require_once 'Processor.php';
class Alpha implements Processor {
    public function process() { echo "Alpha"; }
    public function unusedOp() { echo "u"; }
}
