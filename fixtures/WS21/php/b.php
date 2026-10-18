<?php
namespace App\Models;
class User { function __construct() {} static function make() { return new self(); } static function make2() { return new static(); } }
class Plain { }
