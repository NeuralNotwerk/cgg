pragma solidity ^0.8.0;
struct Pos { uint x; }
library PosLib {
    function shift(Pos memory p, uint d) internal pure returns (uint) { return p.x + d; }
}
library Other {
    function shift(uint a, uint d) internal pure returns (uint) { return a + d; }
}
