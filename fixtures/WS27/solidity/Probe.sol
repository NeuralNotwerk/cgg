contract C {
    function(uint) external stateFn;
    event E(uint ev);
    function f(uint a, function(uint x) external cb) public returns (uint r) {
        uint b = a;
        (uint t, , bool u) = g();
        for (uint i = 0; i < 3; i++) { cb(i); }
        try this.h() returns (uint got) { got; } catch Error(string memory reason) { reason; } catch (bytes memory d) { d; }
        assembly { let y := add(1, 2) function yf(p) -> q { q := p } }
        Token tok = Token(addr);
    }
    modifier m(uint mp) { _; }
    constructor(uint cp) Base(cp) {}
}
