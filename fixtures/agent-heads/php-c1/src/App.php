<?php
namespace App;

use Carbon\Carbon;
use Illuminate\Support\Str;

class App {
    public function run() {
        $d = Carbon::parse("x");
        $c = collect([1]);
        $s = Str::slug("x");
        $j = json_encode([1]);
        $h = \Illuminate\Support\Facades\Http::get("u");
    }
}
