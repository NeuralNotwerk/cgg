pragma solidity ^0.8.0;
interface IShape { function area() external returns (uint); }
abstract contract Base { function perim() public virtual returns (uint); }
contract Sq is Base, IShape { function area() external returns (uint) { return 1; } function perim() public override returns (uint) { return 4; } }
contract Ci is Base, IShape { function area() external returns (uint) { return 3; } function perim() public override returns (uint) { return 6; } }
contract Use { function run(address a) public { IShape(a).area(); holder.item.area(); holder.item.perim(); } }
