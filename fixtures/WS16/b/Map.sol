pragma solidity ^0.8.20;
import {Set} from "./Set.sol";
library Map {
    using Set for *;
    struct AMap { Set.Bytes32Set keys; }
    struct BMap { AMap inner; }
    function contains(AMap storage map, bytes32 key) internal view returns (bool) {
        return map.keys.contains(key);
    }
    function contains(BMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    struct CMap { AMap inner; }
    function contains(CMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    struct DMap { AMap inner; }
    function contains(DMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    struct EMap { AMap inner; }
    function contains(EMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    struct FMap { AMap inner; }
    function contains(FMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    struct GMap { AMap inner; }
    function contains(GMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map.inner, key);
    }
    function tryGet(AMap storage map, bytes32 key) internal view returns (bool) {
        return contains(map, key);
    }
}
