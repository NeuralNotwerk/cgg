<?php
namespace App;
function go() { throw new \Exception("x"); }
function go2() { return new \Lib\Errors\Exception("x"); }
