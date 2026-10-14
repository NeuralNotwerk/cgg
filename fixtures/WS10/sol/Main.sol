pragma solidity ^0.8.0;
import "./Lib.sol";
contract Main {
    using PosLib for Pos;
    function run(Pos memory p) public pure returns (uint) {
        return p.shift(1);
    }
}
