pragma solidity ^0.8.20;
library Set {
    struct Bytes32Set { bytes32[] values; }
    function contains(Bytes32Set storage set, bytes32 value) internal view returns (bool) {
        return set.values.length > 0 && value != 0;
    }
}
