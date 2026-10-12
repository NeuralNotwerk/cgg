<?php
class Foo { public function zorkle() { return 1; } }
function ctlCall(Foo $f) { return $f->zorkle(); }
function nullSafe(?Foo $f) { return $f?->zorkle(); }
