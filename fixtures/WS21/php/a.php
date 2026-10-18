<?php
class Base { function __construct($x) {} }
class Coll extends Base {
  function __construct($y) { parent::__construct($y); }
}
function main() { $c = new Coll(1); }
