<?php
namespace App;
use Acme\Util\Uuid;
use Vendor\Lib\Uuid as VUuid;
use function Acme\Util\helper;
use Acme\Other\Base;
class Main extends Base {
  public function boot() { parent::boot(); }
  public function run() {
    Uuid::fromString('a');
    VUuid::fromString('b');
    helper();
  }
}
