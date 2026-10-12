<?php
require_once 'Processor.php';
class Delta implements Processor {
    public function process() { echo "Delta"; }
    public function unusedOp() { echo "u"; }
}
