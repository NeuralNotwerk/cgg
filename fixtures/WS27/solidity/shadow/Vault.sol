pragma solidity ^0.8.0;

contract Token {
    constructor(address a) {}
}

contract Vault {
    // Functions whose names the parameters and locals below reuse.
    function cb(uint x) internal pure returns (uint) { return x; }
    function handler(uint x) internal pure returns (uint) { return x; }
    function step(uint x) internal pure returns (uint) { return x; }
    function now2() internal pure returns (uint) { return 0; }

    // A function-typed parameter, a local, a `for`-scoped local.
    function withParam(function(uint) internal pure returns (uint) cb) internal pure returns (uint) {
        return cb(1);
    }
    function withLocal() internal pure returns (uint) {
        function(uint) internal pure returns (uint) handler = step;
        return handler(2);
    }
    function withFor(function(uint) internal pure returns (uint)[] memory fs) internal pure {
        for (uint i = 0; i < fs.length; i++) {
            function(uint) internal pure returns (uint) step = fs[i];
            step(3);
        }
        step(4);
    }

    // A member call is never shadowed; the value sees the function; a
    // local named like a contract does not hide its cast.
    function controls(uint now2) internal pure returns (uint) {
        uint v = this.cb2(now2);
        Token Token2 = Token(address(0));
        return v;
    }
    function cb2(uint x) public pure returns (uint) { return cb(x); }
}
