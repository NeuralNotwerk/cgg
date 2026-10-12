<?php
class Repo {
  public function zlist($d) { return $d; }
  public function caller($data) { return zlist($data); }
}
function top($data) { return zlist($data); }
