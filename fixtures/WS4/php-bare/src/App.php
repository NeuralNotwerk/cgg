<?php
namespace App;

use App\Util\Dates;

class App {
    public function run() {
        helper(1);
        collect([1]);
        $this->own();
    }
    public function own() { return 1; }
}
