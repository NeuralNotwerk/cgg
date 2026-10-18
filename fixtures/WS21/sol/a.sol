pragma solidity ^0.8.0;
contract Base {
  constructor(uint x) {}
}
contract Child is Base {
  constructor(uint y) Base(y) {}
}
contract Other is Base(1) {
}
contract Factory {
  function make() public { Child c = new Child(1); }
}
