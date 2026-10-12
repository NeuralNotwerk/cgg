<?php
namespace App;
class App { public function go() { return \App\Jobs\Runner::spawnJob(1); } }
