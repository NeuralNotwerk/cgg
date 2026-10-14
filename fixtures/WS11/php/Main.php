<?php
namespace M;

class Main {
    private ?Foo $f;
    private Foo $g;
    private Foo|null $h;

    public function aControl(Foo $x) {
        $x->run();
    }
    public function bNullable(?Foo $x) {
        $x->run();
    }
    public function cUnionNull(Foo|null $x) {
        $x->run();
    }
    public function dNullFirst(null|Foo $x) {
        $x->run();
    }
    public function eMulti(Foo|Bar $x) {
        $x->run();
    }
    public function fUntyped($x) {
        $x->run();
    }
    public function m1() {
        $this->f->run();
    }
    public function m2() {
        $this->g->run();
    }
    public function m3() {
        $this->h->run();
    }
    public function mk(): ?Foo { return null; }
    public function gRet() {
        $foo = $this->mk();
        $foo->run();
    }
}
