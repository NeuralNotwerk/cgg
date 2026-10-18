<?php
namespace App;
class Text {
    public function strlen($s) { return 1; }
}
function f($s) {
    $t = new Text();
    $n = $t->strlen($s);
    return \strlen($s) + $n;
}
