<?php
interface Shape { public function area(); }
abstract class Base { abstract public function perim(); public function half() { return $this->perim() / 2; } }
class Sq extends Base implements Shape { public function area() { return 1; } public function perim() { return 4; } }
class Ci extends Base implements Shape { public function area() { return 3; } public function perim() { return 6; } }
function run($x) { $x->area(); $x->perim(); }
